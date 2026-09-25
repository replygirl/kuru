use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    ops::Range,
    path::Path,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use kuru_core::{
    Completion, CompletionRequest, Config, ContentBlock, ContextBudget, ContextEstimate,
    ContextSizing, ContextSourceKind, ContextSourceSize, Message, ModelInfo, ModelMetadata,
    ModelRoute, TokenizerEncoding, Usage, advertised_metadata, enrich_model,
    estimated_tokens_for_bytes, tokenizer_for_model,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    auth::{AuthManager, AuthRoute, RequestCredentials},
    http,
    retry::{self, OperationBudget, RetryDecision},
};

mod diagnostics;
mod sse;
#[cfg(test)]
mod subscription_tests;

const SUBSCRIPTION_BASE: &str = "https://chatgpt.com/backend-api/codex";
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);
// This describes the audited catalog wire contract, not Kuru's identity.
const CATALOG_COMPATIBILITY: &str = "0.154.0";

/// Local BPE counts serialized text, while the provider charges for its own
/// request envelope. A small positive reserve covers role and tool framing
/// without claiming to reproduce the provider's undocumented token format.
/// Short native GPT-5.6 calibration samples found that the serialized BPE
/// count itself exceeded reported usage, so a large added margin would discard
/// useful context without improving those observed fits.
fn native_structural_allowance(body: &Value) -> u64 {
    let input_items = body["input"].as_array().map_or(0, Vec::len) as u64;
    let tools = body["tools"].as_array().map_or(0, Vec::len) as u64;
    64u64
        .saturating_add(input_items.saturating_mul(8))
        .saturating_add(tools.saturating_mul(8))
}

fn estimate_native_input(
    route: ModelRoute,
    body: &Value,
    payload: &[u8],
    native_output_ranges: Option<&[Range<usize>]>,
) -> Result<(u64, ContextSizing)> {
    let allowance = native_structural_allowance(body);
    let mapping = tokenizer_for_model(
        route,
        body["model"]
            .as_str()
            .context("Responses body lacks model")?,
    )?;
    if matches!(
        mapping.map(|value| value.value),
        Some(TokenizerEncoding::O200kBase)
    ) {
        let (visible_payload, opaque_bytes, sizing) = if let Some(ranges) = native_output_ranges {
            // Pending native output is kept byte-for-byte in the actual request.
            // Only the local sizing copy excludes those items: Kuru already
            // knows their exact ranges from every saved protocol continuation.
            // The provider's opaque token representation is unknown, so use
            // the prior byte heuristic for that range alone.
            let mut visible = body.clone();
            let items = visible["input"]
                .as_array_mut()
                .context("Responses body input is not an array")?;
            let mut next_end = items.len();
            for range in ranges.iter().rev() {
                ensure!(
                    range.start <= range.end && range.end <= next_end,
                    "native output range exceeds Responses input"
                );
                next_end = range.start;
                items.drain(range.clone());
            }
            let visible_payload = serde_json::to_vec(&visible)?;
            ensure!(
                visible_payload.len() <= payload.len(),
                "native output removal enlarged Responses body"
            );
            let opaque_bytes = (payload.len() - visible_payload.len()) as u64;
            (
                visible_payload,
                opaque_bytes,
                ContextSizing::MixedNativeEstimate,
            )
        } else {
            (payload.to_vec(), 0, ContextSizing::O200kBaseEstimate)
        };
        let serialized =
            std::str::from_utf8(&visible_payload).context("Responses body is not UTF-8")?;
        let visible_tokens = tiktoken_rs::o200k_base_singleton().count_ordinary(serialized) as u64;
        return Ok((
            visible_tokens
                .saturating_add(estimated_tokens_for_bytes(opaque_bytes))
                .saturating_add(allowance),
            sizing,
        ));
    }
    // Unknown routes preserve the historical byte heuristic plus the small
    // structural reserve. Neither term is a guaranteed provider-token bound.
    Ok((
        estimated_tokens_for_bytes(payload.len() as u64).saturating_add(allowance),
        ContextSizing::ConservativeByteFallback,
    ))
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    /// Measure the request that this connector would dispatch without sending
    /// it. Provider-private continuation state remains inside the connector.
    /// Implementations MUST preserve whole-message prefix monotonicity: when
    /// every other request field is fixed, appending one complete message may
    /// not reduce either `final_body_bytes` or `estimated_input_tokens`. This
    /// lets bounded callers find the largest fitting prefix without quadratic
    /// serialization.
    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        estimate_demo_context(request)
    }
    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()>;

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        collect_completion(self, request, None).await
    }
}

/// Pure sizing result for the largest whole oldest source prefix that fits one
/// compaction request. The caller retains the source rows and selects this
/// count; connectors never gain memory or policy authority through sizing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPrefixEstimate {
    pub selected_messages: usize,
    pub estimate: ContextEstimate,
}

/// Measure complete source prefixes without provider dispatch. The base
/// request contains the fixed instruction and prior rolling summary; `source`
/// is ordered oldest first and is never split or reordered.
pub async fn largest_fitting_context_prefix(
    provider: &dyn Provider,
    base: &CompletionRequest,
    source: &[Message],
) -> Result<ContextPrefixEstimate> {
    ensure!(source.len() <= 1_024, "context source exceeds 1024 rows");
    let measure = |count: usize| {
        let mut request = base.clone();
        request.messages.extend_from_slice(&source[..count]);
        async move { provider.estimate_context(&request).await }
    };
    let fits = |estimate: &ContextEstimate| {
        estimate.final_body_bytes <= crate::MAX_BYTES as u64 && estimate.ensure_fits().is_ok()
    };

    let base_estimate = measure(0).await?;
    if source.is_empty() || !fits(&base_estimate) {
        return Ok(ContextPrefixEstimate {
            selected_messages: 0,
            estimate: base_estimate,
        });
    }
    let all = measure(source.len()).await?;
    if fits(&all) {
        return Ok(ContextPrefixEstimate {
            selected_messages: source.len(),
            estimate: all,
        });
    }

    // Serialized request size and the connector estimators grow with a whole
    // appended message. Binary search keeps a 1,024-row/32-MiB storage page
    // from requiring quadratic serialization solely to find its fitting edge.
    let mut low = 0usize;
    let mut high = source.len();
    let mut selected = base_estimate;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        let estimate = measure(middle).await?;
        if fits(&estimate) {
            low = middle;
            selected = estimate;
        } else {
            high = middle;
        }
    }
    Ok(ContextPrefixEstimate {
        selected_messages: low,
        estimate: selected,
    })
}

/// A bounded, protocol-neutral observation from a provider response stream.
///
/// Native item IDs and output indexes are opaque correlation values. They are
/// neither tool-call IDs nor positions in [`Completion::blocks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    ContextMeasured(ContextEstimate),
    TextDelta {
        item_id: String,
        output_index: u64,
        content_index: u64,
        source: TextDeltaSource,
        text: String,
    },
    ReasoningSummaryDelta {
        item_id: String,
        output_index: u64,
        summary_index: u64,
        text: String,
    },
    /// Provider-authored reasoning summaries reconciled against a successful
    /// terminal response. This private sidecar deliberately remains separate
    /// from [`Completion`] and public output blocks. Consumers that persist it
    /// must buffer it until the following [`ProviderEvent::Completed`] has
    /// been accepted, because an observer can still reject that terminal event.
    SettledReasoningSummaries(Vec<ProviderReasoningSummary>),
    ToolCallDelta {
        item_id: String,
        output_index: u64,
        arguments_fragment: String,
    },
    Usage(Usage),
    Completed(Completion),
    Failed {
        kind: ProviderFailureKind,
    },
}

/// One provider-authored reasoning summary from a settled response.
///
/// Item and output coordinates are present only when the provider supplied
/// them. The terminal output array may be reordered, so callers must not
/// manufacture a coordinate when it was absent from the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReasoningSummary {
    pub item_id: Option<String>,
    pub output_index: Option<u64>,
    pub summary_index: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextDeltaSource {
    OutputText,
    Refusal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureKind {
    Incomplete,
    Failed,
    Error,
}

/// Async, fallible callback for provider observations. Implementations must
/// retain bounded state; an error stops the provider request.
pub trait ProviderSink: Send {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

/// Collect one normalized provider stream into its sole final completion.
/// An optional observer sees the same bounded observations while collection
/// keeps terminal and usage rules authoritative.
pub async fn collect_completion<P: Provider + ?Sized>(
    provider: &P,
    request: CompletionRequest,
    observer: Option<&mut dyn ProviderSink>,
) -> Result<Completion> {
    let mut collector = CompletionCollector {
        observer,
        completion: None,
        failed: None,
        usage: Usage::default(),
    };
    provider.stream(request, &mut collector).await?;
    ensure!(
        collector.failed.is_none(),
        "provider stream ended after failure"
    );
    collector
        .completion
        .map(|mut completion| {
            fill_missing_usage(&mut completion.usage, &collector.usage);
            completion
        })
        .context("provider stream ended without response.completed")
}

struct CompletionCollector<'a> {
    observer: Option<&'a mut dyn ProviderSink>,
    completion: Option<Completion>,
    failed: Option<ProviderFailureKind>,
    usage: Usage,
}

impl ProviderSink for CompletionCollector<'_> {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ensure!(
                self.completion.is_none() && self.failed.is_none(),
                "provider stream emitted an event after its terminal outcome"
            );
            if let ProviderEvent::Usage(usage) = &event {
                merge_usage(&mut self.usage, usage);
            }
            match &event {
                ProviderEvent::Completed(completion) => self.completion = Some(completion.clone()),
                ProviderEvent::Failed { kind } => self.failed = Some(*kind),
                _ => {}
            }
            if let Some(observer) = self.observer.as_mut() {
                observer.emit(event).await?;
            }
            Ok(())
        })
    }
}

fn merge_usage(target: &mut Usage, observed: &Usage) {
    if observed.input_tokens.is_some() {
        target.input_tokens = observed.input_tokens;
    }
    if observed.output_tokens.is_some() {
        target.output_tokens = observed.output_tokens;
    }
    if observed.cached_input_tokens.is_some() {
        target.cached_input_tokens = observed.cached_input_tokens;
    }
    if observed.reasoning_output_tokens.is_some() {
        target.reasoning_output_tokens = observed.reasoning_output_tokens;
    }
}

fn fill_missing_usage(target: &mut Usage, observed: &Usage) {
    if target.input_tokens.is_none() {
        target.input_tokens = observed.input_tokens;
    }
    if target.output_tokens.is_none() {
        target.output_tokens = observed.output_tokens;
    }
    if target.cached_input_tokens.is_none() {
        target.cached_input_tokens = observed.cached_input_tokens;
    }
    if target.reasoning_output_tokens.is_none() {
        target.reasoning_output_tokens = observed.reasoning_output_tokens;
    }
}

pub async fn provider(config: &Config, cwd: &Path, data_dir: &Path) -> Result<Arc<dyn Provider>> {
    match config.provider.as_str() {
        "demo" => Ok(Arc::new(DemoProvider)),
        "codex" => {
            let manager = AuthManager::new(data_dir.to_owned(), cwd.to_owned(), None)?;
            let initial = manager.credentials_snapshot().await?;
            Ok(Arc::new(ResponsesProvider::subscription(manager, initial)?))
        }
        "responses" => Ok(Arc::new(ResponsesProvider::new(
            &config.api_base,
            &config.api_key_env,
        )?)),
        other => bail!("unknown provider: {other}"),
    }
}

/// Offline transport for installation checks; never masquerades as inference.
pub struct DemoProvider;

fn estimate_demo_context(request: &CompletionRequest) -> Result<ContextEstimate> {
    for message in &request.messages {
        provider_text(message)?;
    }
    let prompt = json!({
        "instructions": request.instructions,
        "messages": request.messages,
        "tools": request.tools,
    });
    let prompt_bytes = serde_json::to_vec(&prompt)?.len() as u64;
    let effective_budget = request
        .context_budget
        .clone()
        .unwrap_or_else(ContextBudget::legacy_default);
    effective_budget.validate()?;
    Ok(ContextEstimate::for_final_body(
        effective_budget,
        prompt_bytes,
        false,
        vec![ContextSourceSize {
            kind: ContextSourceKind::CurrentInput,
            serialized_bytes: prompt_bytes,
            estimated_tokens: estimated_tokens_for_bytes(prompt_bytes),
            units: 1,
            mandatory: true,
        }],
    ))
}

#[async_trait]
impl Provider for DemoProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "demo".into(),
            name: "Deterministic demo (offline)".into(),
            efforts: vec![],
            default_effort: None,
            metadata: ModelMetadata::default(),
        }])
    }

    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        estimate_demo_context(request)
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        // Demo has no HTTP body, but it is still a harness provider. Measure
        // its effective prompt rather than silently bypassing the same budget.
        let context = self.estimate_context(&request).await?;
        sink.emit(ProviderEvent::ContextMeasured(context.clone()))
            .await?;
        context.ensure_fits()?;
        let latest = request
            .messages
            .iter()
            .rev()
            .filter(|message| message.role == "user")
            .find_map(Message::plain_text)
            .unwrap_or("Ready");
        let original = latest
            .strip_prefix("User request: ")
            .unwrap_or(latest)
            .split("\nExplicit contributions to this speaking identity:")
            .next()
            .unwrap_or(latest);
        let concise: String = original.chars().take(240).collect();
        let text = if request.instructions.contains("Phase: dream:") {
            "[demo] Offline demo memory consolidation.".into()
        } else if request.instructions.contains("Phase: speak and act:") {
            let mode = request
                .actor
                .split('/')
                .rev()
                .nth(2)
                .unwrap_or("configured mode");
            let count = request
                .instructions
                .lines()
                .find_map(|line| line.strip_prefix("Active peers: "))
                .and_then(|line| serde_json::from_str::<Vec<Value>>(line).ok())
                .map(|roster| roster.len())
                .unwrap_or(0);
            format!(
                "[demo] Kuru is running in {mode} with {count} active peers. No model request was sent. Your message: {concise}"
            )
        } else {
            format!("[demo] Offline contribution: {concise}")
        };
        sink.emit(ProviderEvent::TextDelta {
            item_id: "demo-output".into(),
            output_index: 0,
            content_index: 0,
            source: TextDeltaSource::OutputText,
            text: text.clone(),
        })
        .await?;
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            text,
            vec![],
            0,
            0,
        )))
        .await
    }
}

#[derive(Clone)]
struct Pending {
    input: Vec<Value>,
    /// Exact positions of older native outputs already retained in `input`.
    native_output_ranges: Vec<Range<usize>>,
    output: Vec<Value>,
    calls: Vec<String>,
}

/// Standard Responses API transport. Pending native output (including encrypted
/// reasoning) is transient and scoped to exactly one actor, solely to complete
/// function-call protocol handshakes. Kuru remains the durable memory owner.
pub struct ResponsesProvider {
    base: String,
    auth: Authentication,
    client: reqwest::Client,
    completion_timeout: Duration,
    actors: Mutex<BTreeMap<String, Arc<Mutex<Option<Pending>>>>>,
}

enum Authentication {
    Environment(String),
    Subscription {
        manager: AuthManager,
        initial: RequestCredentials,
    },
}

impl ResponsesProvider {
    pub fn new(base: &str, key_env: &str) -> Result<Self> {
        http::endpoint(base).map_err(|_| diagnostics::invalid_endpoint())?;
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            auth: Authentication::Environment(key_env.into()),
            client: provider_client(COMPLETION_TIMEOUT)?,
            completion_timeout: COMPLETION_TIMEOUT,
            actors: Mutex::new(BTreeMap::new()),
        })
    }

    fn subscription(manager: AuthManager, initial: RequestCredentials) -> Result<Self> {
        ensure!(
            initial.route() == AuthRoute::Chatgpt,
            "subscription authentication required"
        );
        Ok(Self {
            base: SUBSCRIPTION_BASE.into(),
            auth: Authentication::Subscription { manager, initial },
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(COMPLETION_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .map_err(|_| diagnostics::client())?,
            completion_timeout: COMPLETION_TIMEOUT,
            actors: Mutex::new(BTreeMap::new()),
        })
    }

    fn is_subscription(&self) -> bool {
        matches!(self.auth, Authentication::Subscription { .. })
    }

    fn model_route(&self) -> ModelRoute {
        if self.is_subscription() {
            ModelRoute::CodexSubscription
        } else if self.base == "https://api.openai.com/v1" {
            ModelRoute::OpenAiResponses
        } else {
            ModelRoute::CustomResponses
        }
    }

    fn enrich_models(&self, models: Vec<ModelInfo>) -> Result<Vec<ModelInfo>> {
        models
            .into_iter()
            .map(|model| enrich_model(self.model_route(), model))
            .collect()
    }

    fn operation(&self, catalog: bool) -> diagnostics::Operation {
        match (self.is_subscription(), catalog) {
            (false, false) => diagnostics::Operation::ResponsesCompletion,
            (false, true) => diagnostics::Operation::ResponsesCatalog,
            (true, false) => diagnostics::Operation::ChatgptCompletion,
            (true, true) => diagnostics::Operation::ChatgptCatalog,
        }
    }

    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
        operation: diagnostics::Operation,
        budget: &OperationBudget,
    ) -> Result<reqwest::Response> {
        let started = Instant::now();
        let environment = match &self.auth {
            Authentication::Environment(key_env) if !key_env.is_empty() => {
                Some(environment_value(environment_key(key_env)?)?)
            }
            _ => None,
        };
        let mut credentials = match &self.auth {
            Authentication::Subscription { manager, initial } => {
                let observed = manager.credentials_snapshot().await?;
                same_session(initial, &observed)?;
                let current = manager.resolve_for_operation(&observed, budget).await?;
                Some(current)
            }
            Authentication::Environment(_) => None,
        };
        let template = builder
            .try_clone()
            .context("provider request is not replayable")?;
        let mut rotated = false;
        loop {
            budget.take_provider_send()?;
            let mut request = template
                .try_clone()
                .context("provider request is not replayable")?;
            if let Some(key) = &environment {
                request = request.bearer_auth(key);
            }
            if let Some(current) = &credentials {
                request = subscription_headers(
                    request
                        .header(
                            reqwest::header::USER_AGENT,
                            concat!("Kuru/", env!("CARGO_PKG_VERSION")),
                        )
                        .header("originator", "kuru"),
                    current,
                )?;
            }
            let response = request
                .send()
                .await
                .map_err(|error| diagnostics::transport(operation, error))?;
            if response.status().is_success() {
                return Ok(response);
            }
            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                && credentials.is_some()
                && !rotated
            {
                let current = credentials.as_ref().unwrap();
                let Authentication::Subscription { manager, initial } = &self.auth else {
                    unreachable!();
                };
                drop(response);
                let allowance = budget.begin_rotation(crate::IO_TIMEOUT)?;
                let refreshed = manager.refresh_with_allowance(current, allowance).await?;
                same_session(initial, &refreshed)?;
                credentials = Some(refreshed);
                rotated = true;
                continue;
            }
            let retry_after = retry::retry_after(response.headers());
            let status = response.status().as_u16();
            let rejected = diagnostics::rejected(response, operation).await;
            if !rejected.retryable {
                tracing::info!(target: "kuru.provider", operation = operation.tracing_label(), attempt = budget.provider_attempts(), status = status, elapsed_ms = started.elapsed().as_millis() as u64, "provider retry terminal");
                return Err(rejected.error);
            }
            match budget.retry_delay(retry_after.as_ref(), std::time::SystemTime::now()) {
                RetryDecision::Delay(delay) => {
                    tracing::info!(target: "kuru.provider", operation = operation.tracing_label(), attempt = budget.provider_attempts(), status = status, delay_ms = delay.as_millis() as u64, elapsed_ms = started.elapsed().as_millis() as u64, "provider retry scheduled");
                    tokio::time::sleep(delay).await
                }
                RetryDecision::Exhausted => {
                    tracing::info!(target: "kuru.provider", operation = operation.tracing_label(), attempt = budget.provider_attempts(), status = status, elapsed_ms = started.elapsed().as_millis() as u64, "provider retry exhausted");
                    return Err(retry::exhausted(operation, budget.provider_attempts()));
                }
            }
        }
    }

    async fn actor(&self, name: &str) -> Result<Arc<Mutex<Option<Pending>>>> {
        let mut actors = self.actors.lock().await;
        ensure!(
            actors.contains_key(name) || actors.len() < 1024,
            "provider actor transport limit reached; start a new session"
        );
        Ok(actors
            .entry(name.into())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone())
    }
}

fn provider_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| diagnostics::client())
}

fn environment_key(name: &str) -> Result<String> {
    std::env::var(name).map_err(environment_error)
}

fn environment_value(key: String) -> Result<String> {
    ensure!(
        !key.trim().is_empty(),
        "Responses API authentication environment variable is empty"
    );
    Ok(key)
}

fn environment_error(error: std::env::VarError) -> anyhow::Error {
    match error {
        std::env::VarError::NotPresent => {
            anyhow::Error::msg("Responses API authentication environment variable is not set")
        }
        std::env::VarError::NotUnicode(_) => anyhow::Error::msg(
            "Responses API authentication environment variable is not valid Unicode",
        ),
    }
}

fn same_session(initial: &RequestCredentials, current: &RequestCredentials) -> Result<()> {
    ensure!(
        current.route() == AuthRoute::Chatgpt
            && initial.account_id() == current.account_id()
            && initial.session_id() == current.session_id(),
        "ChatGPT account or login session changed; start a new Kuru session"
    );
    Ok(())
}

fn subscription_headers(
    builder: reqwest::RequestBuilder,
    credentials: &RequestCredentials,
) -> Result<reqwest::RequestBuilder> {
    let account = credentials
        .account_id()
        .context("ChatGPT credentials lack account identity")?;
    ensure!(
        !account.is_empty(),
        "ChatGPT credentials lack account identity"
    );
    Ok(builder
        .bearer_auth(credentials.bearer())
        .header("ChatGPT-Account-ID", account))
}

fn provider_text(message: &Message) -> Result<String> {
    if let Some(text) = message.plain_text() {
        return Ok(text.into());
    }
    ensure!(
        !message.blocks.iter().any(|block| matches!(
            block,
            ContentBlock::Image { .. } | ContentBlock::CacheBoundary { .. }
        )),
        "Responses provider does not support image or cache content blocks"
    );
    serde_json::to_string(&message.blocks).context("encode typed message for Responses provider")
}

fn text_message(message: &Message) -> Result<Value> {
    let content = provider_text(message)?;
    match message.role.as_str() {
        "assistant" | "system" | "developer" | "user" => {
            Ok(json!({"role":message.role,"content":content}))
        }
        _ => Ok(json!({"role":"user","content":format!("[{} record]\n{content}", message.role)})),
    }
}

struct SelectedInput {
    items: Vec<Value>,
    /// `Some` means a live native tool continuation; each range is an exact
    /// saved provider-output segment in `items`, including prior hops.
    native_output_ranges: Option<Vec<Range<usize>>>,
}

fn input_items_selected(
    messages: &[Message],
    pending: Option<&Pending>,
    current_message_count: Option<usize>,
) -> Result<SelectedInput> {
    // Validate every block before selecting native continuation records: a
    // matching tool receipt must not let an earlier unsupported block vanish
    // from validation and reach a provider dispatch.
    for message in messages {
        provider_text(message)?;
    }
    // An absent boundary is a legacy/direct request. Never infer native
    // continuation from historical text or receipt adjacency.
    let Some(current_message_count) = current_message_count else {
        return Ok(SelectedInput {
            items: messages.iter().map(text_message).collect::<Result<_>>()?,
            native_output_ranges: None,
        });
    };
    ensure!(
        current_message_count <= messages.len(),
        "current model input boundary exceeds message history"
    );
    let Some(pending) = pending else {
        return Ok(SelectedInput {
            items: messages.iter().map(text_message).collect::<Result<_>>()?,
            native_output_ranges: None,
        });
    };
    let current = &messages[messages.len() - current_message_count..];
    let mut found = BTreeMap::new();
    let mut other_current = Vec::new();
    let mut invalid_tool = false;
    for message in current {
        if message.role != "tool" {
            other_current.push(text_message(message)?);
            continue;
        }
        let receipt = match message.blocks.as_slice() {
            [
                ContentBlock::ToolResult {
                    call_id, output, ..
                },
            ] => Some((call_id.clone(), output.clone())),
            // Legacy string-encoded receipts are accepted only at this live
            // continuation seam. Durable history is never guessed from JSON text.
            [ContentBlock::Text { text }] => {
                serde_json::from_str::<Value>(text).ok().and_then(|value| {
                    value["call_id"]
                        .as_str()
                        .map(str::to_owned)
                        .zip(value.get("output").cloned())
                })
            }
            _ => None,
        };
        if let Some((id, output)) = receipt {
            ensure!(
                pending.calls.iter().any(|call| call == &id),
                "tool receipt does not match a pending Responses call"
            );
            ensure!(
                found.insert(id.to_owned(), output).is_none(),
                "duplicate tool receipt for pending Responses call"
            );
        } else {
            invalid_tool = true;
        }
    }
    ensure!(
        !invalid_tool,
        "invalid current tool receipt for pending Responses call"
    );
    // A fresh user turn has no current receipt and starts a new protocol
    // context. Historical receipts remain ordinary safe history.
    if found.is_empty() {
        return Ok(SelectedInput {
            items: messages.iter().map(text_message).collect::<Result<_>>()?,
            native_output_ranges: None,
        });
    }
    ensure!(
        found.len() == pending.calls.len(),
        "missing function outputs for pending Responses calls"
    );
    let mut input = pending.input.clone();
    let mut ranges = pending.native_output_ranges.clone();
    let output_start = input.len();
    input.extend(pending.output.clone());
    ranges.push(output_start..input.len());
    for call in &pending.calls {
        let output = &found[call];
        input.push(json!({"type":"function_call_output","call_id":call,"output":output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string())}));
    }
    input.extend(other_current);
    Ok(SelectedInput {
        items: input,
        native_output_ranges: Some(ranges),
    })
}

struct PreparedResponsesRequest {
    input: Vec<Value>,
    native_output_ranges: Option<Vec<Range<usize>>>,
    body: Value,
    payload: Vec<u8>,
    context: ContextEstimate,
}

fn prepare_responses_request(
    route: ModelRoute,
    subscription: bool,
    request: &CompletionRequest,
    pending: Option<&Pending>,
) -> Result<PreparedResponsesRequest> {
    ensure!(
        request.model != "auto",
        "select an explicit model for the Responses provider"
    );
    let SelectedInput {
        items: input,
        native_output_ranges,
    } = input_items_selected(&request.messages, pending, request.current_message_count)?;
    let native_continuation_mandatory = native_output_ranges.is_some();
    let input_bytes = serde_json::to_vec(&input)?.len() as u64;
    let mut tools = request
        .tools
        .iter()
        .map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters,"strict":false}))
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| {
        left["name"]
            .as_str()
            .cmp(&right["name"].as_str())
            .then_with(|| left.to_string().cmp(&right.to_string()))
    });
    let mut body = json!({"model":request.model,"instructions":request.instructions,"input":input,"store":false,"include":["reasoning.encrypted_content"],"tools":tools});
    if let Some(effort) = &request.effort {
        body["reasoning"] = json!({"effort":effort});
    }
    body["stream"] = json!(true);
    if subscription {
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(true);
    }
    let payload = serde_json::to_vec(&body)?;
    let instructions_bytes = serde_json::to_vec(&body["instructions"])?.len() as u64;
    let tools_bytes = serde_json::to_vec(&body["tools"])?.len() as u64;
    let source = |kind, serialized_bytes, units, mandatory| ContextSourceSize {
        kind,
        serialized_bytes,
        estimated_tokens: estimated_tokens_for_bytes(serialized_bytes),
        units,
        mandatory,
    };
    let accounted = instructions_bytes
        .saturating_add(tools_bytes)
        .saturating_add(input_bytes);
    let effective_budget = request
        .context_budget
        .clone()
        .unwrap_or_else(ContextBudget::legacy_default);
    effective_budget.validate()?;
    let (estimated_input_tokens, sizing) =
        estimate_native_input(route, &body, &payload, native_output_ranges.as_deref())?;
    let context = ContextEstimate::from_measured_final_body(
        effective_budget,
        payload.len() as u64,
        estimated_input_tokens,
        sizing,
        native_continuation_mandatory,
        vec![
            source(
                ContextSourceKind::SelectedInstructions,
                instructions_bytes,
                1,
                false,
            ),
            source(
                ContextSourceKind::ToolSchemas,
                tools_bytes,
                request.tools.len() as u64,
                true,
            ),
            source(
                if native_continuation_mandatory {
                    ContextSourceKind::NativeContinuation
                } else {
                    ContextSourceKind::SelectedInput
                },
                input_bytes,
                body["input"].as_array().map_or(0, Vec::len) as u64,
                native_continuation_mandatory,
            ),
            source(
                ContextSourceKind::WireOverhead,
                (payload.len() as u64).saturating_sub(accounted),
                1,
                true,
            ),
        ],
    );
    Ok(PreparedResponsesRequest {
        input,
        native_output_ranges,
        body,
        payload,
        context,
    })
}

#[cfg(test)]
fn input_items(
    messages: &[Message],
    pending: Option<&Pending>,
    current_message_count: Option<usize>,
) -> Result<Vec<Value>> {
    input_items_selected(messages, pending, current_message_count).map(|selected| selected.items)
}

fn completion(value: &Value, operation: diagnostics::Operation) -> Result<Completion> {
    diagnostics::envelope(value, operation)?;
    let output = value["output"]
        .as_array()
        .context("Responses response lacks output array")?;
    let mut blocks = Vec::new();
    let mut ids = BTreeSet::new();
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                if let Some(content) = item["content"].as_array() {
                    for part in content {
                        if let Some(value) =
                            part["text"].as_str().or_else(|| part["refusal"].as_str())
                        {
                            blocks.push(ContentBlock::Text {
                                text: value.to_owned(),
                            });
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = item["call_id"]
                    .as_str()
                    .context("function call lacks call_id")?;
                ensure!(ids.insert(id), "duplicate function call ID");
                let name = item["name"].as_str().context("function call lacks name")?;
                let arguments = serde_json::from_str(
                    item["arguments"]
                        .as_str()
                        .context("function call lacks arguments")?,
                )
                .map_err(|_| diagnostics::function_arguments(operation))?;
                blocks.push(ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            _ => {}
        }
    }
    Ok(Completion {
        blocks,
        usage: usage(value),
        stop_reason: None,
    })
}

fn usage(value: &Value) -> Usage {
    Usage {
        input_tokens: value["usage"]["input_tokens"].as_u64(),
        output_tokens: value["usage"]["output_tokens"].as_u64(),
        cached_input_tokens: value["usage"]["input_tokens_details"]["cached_tokens"].as_u64(),
        reasoning_output_tokens: value["usage"]["output_tokens_details"]["reasoning_tokens"]
            .as_u64(),
    }
}

#[async_trait]
impl Provider for ResponsesProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let budget = OperationBudget::new(crate::IO_TIMEOUT);
        tokio::time::timeout(crate::IO_TIMEOUT, async {
            let operation = self.operation(true);
            let mut url = reqwest::Url::parse(&format!("{}/models", self.base))
                .map_err(|_| diagnostics::invalid_endpoint())?;
            if self.is_subscription() {
                url.query_pairs_mut()
                    .append_pair("client_version", CATALOG_COMPATIBILITY);
            }
            let builder = self.client.get(url).timeout(crate::IO_TIMEOUT);
            let value =
                diagnostics::json(self.send(builder, operation, &budget).await?, operation).await?;
            if self.is_subscription() {
                return self.enrich_models(subscription_models(&value)?);
            }
            let data = value["data"]
                .as_array()
                .context("models response lacks data array")?;
            data.iter()
                .map(|model| {
                    let id = model["id"].as_str().context("model lacks ID")?;
                    // The public /models API does not advertise reasoning effort. Never
                    // infer a static effort catalog or filter away future model names.
                    Ok(ModelInfo {
                        id: id.into(),
                        name: id.into(),
                        efforts: vec![],
                        default_effort: None,
                        metadata: advertised_model_metadata(model),
                    })
                })
                .collect::<Result<Vec<_>>>()
                .and_then(|models| self.enrich_models(models))
        })
        .await
        .context("model catalog exceeded 60-second total limit")?
    }

    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        let actor = self.actor(&request.actor).await?;
        let pending = actor.lock().await;
        Ok(prepare_responses_request(
            self.model_route(),
            self.is_subscription(),
            request,
            pending.as_ref(),
        )?
        .context)
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let budget = OperationBudget::new(self.completion_timeout);
        let settled = tokio::time::timeout(
            self.completion_timeout,
            self.complete_request(request, &budget, sink),
        )
        .await
        .context("Responses request exceeded 600-second total limit")??;
        if !settled.reasoning_summaries.is_empty() {
            sink.emit(ProviderEvent::SettledReasoningSummaries(
                settled.reasoning_summaries,
            ))
            .await?;
        }
        sink.emit(ProviderEvent::Completed(settled.completion))
            .await
    }
}

fn subscription_models(value: &Value) -> Result<Vec<ModelInfo>> {
    value["models"]
        .as_array()
        .context("ChatGPT models response lacks models array")?
        .iter()
        .map(|model| {
            let id = model["slug"].as_str().context("ChatGPT model lacks slug")?;
            ensure!(!id.is_empty(), "ChatGPT model has empty slug");
            let efforts = match model.get("supported_reasoning_levels") {
                None | Some(Value::Null) => vec![],
                Some(value) => value
                    .as_array()
                    .context("invalid model reasoning levels")?
                    .iter()
                    .map(|level| {
                        level["effort"]
                            .as_str()
                            .context("invalid reasoning effort")
                            .map(str::to_owned)
                    })
                    .collect::<Result<Vec<_>>>()?,
            };
            let default_effort = match model.get("default_reasoning_level") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .context("invalid default reasoning effort")?
                        .to_owned(),
                ),
            };
            Ok(ModelInfo {
                id: id.into(),
                name: model["display_name"].as_str().unwrap_or(id).into(),
                efforts,
                default_effort,
                metadata: advertised_model_metadata(model),
            })
        })
        .collect()
}

fn advertised_model_metadata(model: &Value) -> ModelMetadata {
    let mut metadata = advertised_metadata(
        model["context_window"].as_u64(),
        model["max_output_tokens"].as_u64(),
        model
            .get("capabilities")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|capabilities| capabilities.iter())
            .filter_map(|(name, value)| value.as_bool().map(|value| (name.clone(), value))),
    );
    metadata.extended_context_window_tokens = model["max_context_window"]
        .as_u64()
        .filter(|value| *value > 0)
        .map(kuru_core::Sourced::advertised);
    metadata
}

impl ResponsesProvider {
    async fn complete_request(
        &self,
        request: CompletionRequest,
        budget: &OperationBudget,
        sink: &mut dyn ProviderSink,
    ) -> Result<SettledCompletion> {
        let actor = self.actor(&request.actor).await?;
        let mut pending = actor.lock().await;
        let PreparedResponsesRequest {
            input,
            native_output_ranges,
            body,
            payload,
            context,
        } = prepare_responses_request(
            self.model_route(),
            self.is_subscription(),
            &request,
            pending.as_ref(),
        )?;
        ensure!(
            payload.len() <= crate::MAX_BYTES,
            "Responses request exceeds 2 MiB transport limit; shorten actor context"
        );
        tracing::debug!(
            target: "kuru.provider",
            stage = "context_estimate",
            status = context.sizing.label(),
            input_tokens = context.estimated_input_tokens,
            allowance_tokens = native_structural_allowance(&body),
            bytes = context.final_body_bytes,
            "native input estimate"
        );
        sink.emit(ProviderEvent::ContextMeasured(context.clone()))
            .await?;
        context.ensure_fits()?;
        let mut builder = self
            .client
            .post(format!("{}/responses", self.base))
            .timeout(self.completion_timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        builder = builder.header(reqwest::header::ACCEPT, "text/event-stream");
        let operation = self.operation(false);
        let response = self.send(builder, operation, budget).await?;
        let response = sse::response(
            response,
            crate::IO_TIMEOUT,
            operation,
            self.is_subscription(),
            sink,
        )
        .await?;
        let result = completion(&response.value, operation)?;
        if let Some(input_tokens) = result.usage.input_tokens {
            tracing::debug!(
                target: "kuru.provider",
                stage = "provider_usage",
                input_tokens,
                cached_input_tokens = result.usage.cached_input_tokens.unwrap_or(0),
                status = if result.usage.cached_input_tokens.is_some() { "cached_reported" } else { "cached_unknown" },
                "native provider usage"
            );
        }
        *pending = if result.calls().is_empty() {
            None
        } else {
            Some(Pending {
                input,
                native_output_ranges: native_output_ranges.unwrap_or_default(),
                output: response.value["output"]
                    .as_array()
                    .context("missing output")?
                    .clone(),
                calls: result.calls().into_iter().map(|call| call.id).collect(),
            })
        };
        Ok(SettledCompletion {
            completion: result,
            reasoning_summaries: response.reasoning_summaries,
        })
    }
}

struct SettledCompletion {
    completion: Completion,
    reasoning_summaries: Vec<ProviderReasoningSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply, request};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EstimateOnlyProvider {
        dispatches: AtomicUsize,
    }

    #[async_trait]
    impl Provider for EstimateOnlyProvider {
        async fn models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }

        async fn stream(&self, _: CompletionRequest, _: &mut dyn ProviderSink) -> Result<()> {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            bail!("estimate-only provider must not dispatch")
        }
    }

    #[tokio::test]
    async fn pure_context_estimates_preserve_native_mapping_without_dispatch() {
        let mapped = ResponsesProvider::new("https://api.openai.com/v1", "UNUSED").unwrap();
        let fallback = ResponsesProvider::new("https://provider.invalid/v1", "UNUSED").unwrap();
        let mut input = request();
        input.model = "gpt-5.6-sol".into();
        let mapped_initial = mapped.estimate_context(&input).await.unwrap();
        let fallback_initial = fallback.estimate_context(&input).await.unwrap();
        assert_eq!(mapped_initial.sizing, ContextSizing::O200kBaseEstimate);
        assert_eq!(
            fallback_initial.sizing,
            ContextSizing::ConservativeByteFallback
        );
        assert_eq!(
            mapped_initial.final_body_bytes,
            fallback_initial.final_body_bytes
        );
        let mut previous = [mapped_initial, fallback_initial];
        for suffix in ["a", "東京", "{}[]()", "longer source row"] {
            input.messages.push(Message::text("user", suffix));
            for (index, provider) in [&mapped as &dyn Provider, &fallback as &dyn Provider]
                .into_iter()
                .enumerate()
            {
                let estimate = provider.estimate_context(&input).await.unwrap();
                assert!(estimate.final_body_bytes >= previous[index].final_body_bytes);
                assert!(estimate.estimated_input_tokens >= previous[index].estimated_input_tokens);
                previous[index] = estimate;
            }
        }
    }

    #[tokio::test]
    async fn pure_prefix_estimate_selects_whole_oldest_rows_before_dispatch() {
        let provider = EstimateOnlyProvider {
            dispatches: AtomicUsize::new(0),
        };
        let mut base = request();
        base.model = "demo".into();
        base.instructions = "Produce one bounded rolling context summary.".into();
        base.messages = vec![Message::text("assistant", "prior rolling summary")];
        base.current_message_count = None;
        base.tools.clear();
        base.context_budget = Some(
            ContextBudget::resolve(kuru_core::Sourced::built_in(1_000_000), None, Some(16))
                .unwrap(),
        );
        let source = [
            Message::text("user", "oldest ".repeat(96)),
            Message::text("assistant", "middle ".repeat(96)),
            Message::text("user", "newest ".repeat(96)),
        ];
        let mut two = base.clone();
        two.messages.extend_from_slice(&source[..2]);
        let two_estimate = provider.estimate_context(&two).await.unwrap();
        base.context_budget = Some(
            ContextBudget::resolve(
                kuru_core::Sourced::built_in(
                    two_estimate.estimated_input_tokens.saturating_add(16),
                ),
                None,
                Some(16),
            )
            .unwrap(),
        );
        let fit = largest_fitting_context_prefix(&provider, &base, &source)
            .await
            .unwrap();
        assert_eq!(fit.selected_messages, 2);
        fit.estimate.ensure_fits().unwrap();
        let mut overflow = base.clone();
        overflow.messages.extend_from_slice(&source);
        let overflow = provider.estimate_context(&overflow).await.unwrap();
        assert!(overflow.ensure_fits().is_err());
        assert!(
            kuru_core::ContextCompactionPolicy {
                threshold_percent: 75,
                output_reserve_tokens: 16,
            }
            .threshold_reached(&overflow)
            .unwrap(),
            "the full pre-omission candidate must expose the threshold"
        );

        let mut previous = provider.estimate_context(&base).await.unwrap();
        let mut brute_force = 0;
        for count in 1..=source.len() {
            let mut candidate = base.clone();
            candidate.messages.extend_from_slice(&source[..count]);
            let estimate = provider.estimate_context(&candidate).await.unwrap();
            assert!(estimate.final_body_bytes >= previous.final_body_bytes);
            assert!(estimate.estimated_input_tokens >= previous.estimated_input_tokens);
            if estimate.ensure_fits().is_ok()
                && estimate.final_body_bytes <= crate::MAX_BYTES as u64
            {
                brute_force = count;
            }
            previous = estimate;
        }
        assert_eq!(fit.selected_messages, brute_force);

        let base_estimate = provider.estimate_context(&base).await.unwrap();
        let mut base_overflow = base.clone();
        base_overflow.context_budget =
            Some(ContextBudget::resolve(kuru_core::Sourced::built_in(1), None, Some(1)).unwrap());
        assert_eq!(
            largest_fitting_context_prefix(&provider, &base_overflow, &source)
                .await
                .unwrap()
                .selected_messages,
            0
        );
        let mut one_row_overflow = base.clone();
        one_row_overflow.context_budget = Some(
            ContextBudget::resolve(
                kuru_core::Sourced::built_in(
                    base_estimate.estimated_input_tokens.saturating_add(16),
                ),
                None,
                Some(16),
            )
            .unwrap(),
        );
        assert_eq!(
            largest_fitting_context_prefix(&provider, &one_row_overflow, &source[..1])
                .await
                .unwrap()
                .selected_messages,
            0
        );
        assert_eq!(provider.dispatches.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn final_wire_tokenizer_eligibility_and_fallback_are_explicit() {
        // OpenAI Cookbook, How_to_count_tokens_with_tiktoken.ipynb, checked
        // 2026-09-22: these o200k_base text examples have six and seven tokens.
        let tokenizer = tiktoken_rs::o200k_base_singleton();
        assert_eq!(tokenizer.count_ordinary("tiktoken is great!"), 6);
        assert_eq!(tokenizer.count_ordinary("2 + 2 = 4"), 7);
        for input in [
            "こんにちは、世界。مرحبا بالعالم",
            "fn answer(input: &[u8]) -> Result<Vec<u8>> { Ok(input.to_vec()) }",
            "{\"tools\":[{\"name\":\"file_read\",\"parameters\":{\"type\":\"object\"}}]}",
        ] {
            let body = json!({"model":"gpt-5.6-sol","instructions":input,"input":[{"role":"user","content":input}],"tools":[]});
            let payload = serde_json::to_vec(&body).unwrap();
            let allowance = native_structural_allowance(&body);
            let expected =
                tokenizer.count_ordinary(std::str::from_utf8(&payload).unwrap()) as u64 + allowance;
            assert_eq!(
                estimate_native_input(ModelRoute::OpenAiResponses, &body, &payload, None).unwrap(),
                (expected, ContextSizing::O200kBaseEstimate)
            );
            assert_eq!(
                estimate_native_input(ModelRoute::CustomResponses, &body, &payload, None).unwrap(),
                (
                    estimated_tokens_for_bytes(payload.len() as u64) + allowance,
                    ContextSizing::ConservativeByteFallback
                )
            );
            let unknown_body = json!({"model":"gpt-6-astra","instructions":input,"input":[{"role":"user","content":input}],"tools":[]});
            let unknown_payload = serde_json::to_vec(&unknown_body).unwrap();
            assert_eq!(
                estimate_native_input(
                    ModelRoute::OpenAiResponses,
                    &unknown_body,
                    &unknown_payload,
                    None
                )
                .unwrap(),
                (
                    estimated_tokens_for_bytes(unknown_payload.len() as u64)
                        + native_structural_allowance(&unknown_body),
                    ContextSizing::ConservativeByteFallback
                )
            );
        }
    }

    #[test]
    fn mapped_native_output_uses_mixed_estimate_without_changing_wire_body() {
        let opaque = "encrypted-native-output".repeat(128);
        let body = json!({
            "model": "gpt-5.6-terra",
            "instructions": "Shared instructions",
            "input": [
                {"role":"user","content":"こんにちは fn main() {}"},
                {"type":"reasoning","encrypted_content":opaque},
                {"type":"function_call","call_id":"call-1","name":"file_read","arguments":"{}"},
                {"type":"function_call_output","call_id":"call-1","output":"done"}
            ],
            "tools": []
        });
        let payload = serde_json::to_vec(&body).unwrap();
        let original = payload.clone();
        let mut visible = body.clone();
        visible["input"].as_array_mut().unwrap().drain(1..3);
        let visible_payload = serde_json::to_vec(&visible).unwrap();
        let removed_bytes = (payload.len() - visible_payload.len()) as u64;
        let expected = tiktoken_rs::o200k_base_singleton()
            .count_ordinary(std::str::from_utf8(&visible_payload).unwrap())
            as u64
            + estimated_tokens_for_bytes(removed_bytes)
            + native_structural_allowance(&body);
        let native_range = 1..3;
        assert_eq!(
            estimate_native_input(
                ModelRoute::CodexSubscription,
                &body,
                &payload,
                Some(std::slice::from_ref(&native_range))
            )
            .unwrap(),
            (expected, ContextSizing::MixedNativeEstimate)
        );
        assert_eq!(payload, original, "sizing must leave the wire bytes intact");
        assert_eq!(body["input"][1]["encrypted_content"], opaque);
        assert!(
            expected
                < estimated_tokens_for_bytes(payload.len() as u64)
                    + native_structural_allowance(&body),
            "this opaque fixture should recover usable context over whole-body fallback"
        );
    }

    #[tokio::test]
    async fn captured_custom_wire_matches_preflight_and_stable_tool_order() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"one"}]}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"two"}]}]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut first = request();
        first.model = "gpt-5.6-sol".into();
        first.messages = vec![Message::text("user", "こんにちは fn main() {}")];
        first.current_message_count = Some(1);
        first.tools.push(kuru_core::ToolSpec {
            name: "alpha".into(),
            description: "First".into(),
            parameters: json!({"type":"object"}),
        });
        let mut second = first.clone();
        second.actor = "other/part".into();
        second.tools.reverse();
        let mut first_events = Events::default();
        provider.stream(first, &mut first_events).await.unwrap();
        let mut second_events = Events::default();
        provider.stream(second, &mut second_events).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent[0].body["tools"], sent[1].body["tools"]);
        assert_eq!(sent[0].body["tools"][0]["name"], "alpha");
        for (recorded, events) in sent.iter().zip([first_events, second_events]) {
            let Some(ProviderEvent::ContextMeasured(measured)) = events.0.first() else {
                panic!("expected measured context before provider response");
            };
            let payload = serde_json::to_vec(&recorded.body).unwrap();
            assert_eq!(measured.final_body_bytes, payload.len() as u64);
            assert_eq!(measured.sizing, ContextSizing::ConservativeByteFallback);
            assert_eq!(
                measured.estimated_input_tokens,
                estimated_tokens_for_bytes(payload.len() as u64)
                    + native_structural_allowance(&recorded.body)
            );
        }
    }

    #[tokio::test]
    async fn http_usage_keeps_missing_zero_and_positive_cached_input_distinct() {
        let replies = [
            json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"absent"}]}],"usage":{"input_tokens":20,"output_tokens":2}}),
            json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"zero"}]}],"usage":{"input_tokens":20,"input_tokens_details":{"cached_tokens":0},"output_tokens":2}}),
            json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"positive"}]}],"usage":{"input_tokens":20,"input_tokens_details":{"cached_tokens":8},"output_tokens":2}}),
        ];
        let peer = HttpFixture::new(replies.into_iter().map(Reply::json).collect()).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        for expected in [None, Some(0), Some(8)] {
            let completed = provider.complete(request()).await.unwrap();
            assert_eq!(completed.usage.input_tokens, Some(20));
            assert_eq!(completed.usage.cached_input_tokens, expected);
        }
        assert_eq!(peer.requests.lock().await.len(), 3);
    }

    struct ScriptedProvider(Vec<ProviderEvent>);
    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
        async fn stream(&self, _: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
            for event in &self.0 {
                sink.emit(event.clone()).await?;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct Events(Vec<ProviderEvent>);
    impl ProviderSink for Events {
        fn emit<'a>(
            &'a mut self,
            event: ProviderEvent,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.0.push(event);
                Ok(())
            })
        }
    }

    struct ChannelEvents(tokio::sync::mpsc::Sender<ProviderEvent>);
    impl ProviderSink for ChannelEvents {
        fn emit<'a>(
            &'a mut self,
            event: ProviderEvent,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.0
                    .send(event)
                    .await
                    .map_err(|_| anyhow::anyhow!("test observer closed"))
            })
        }
    }

    #[tokio::test]
    async fn collector_preserves_raw_terminal_usage_for_observers() {
        let provider = ScriptedProvider(vec![
            ProviderEvent::Usage(Usage {
                input_tokens: Some(7),
                cached_input_tokens: Some(0),
                ..Usage::default()
            }),
            ProviderEvent::Completed(Completion {
                blocks: vec![],
                usage: Usage {
                    input_tokens: Some(8),
                    output_tokens: Some(2),
                    ..Usage::default()
                },
                stop_reason: None,
            }),
        ]);
        let mut observer = Events::default();
        let completion = collect_completion(&provider, request(), Some(&mut observer))
            .await
            .unwrap();
        assert_eq!(completion.usage.input_tokens, Some(8));
        assert_eq!(completion.usage.cached_input_tokens, Some(0));
        assert_eq!(completion.usage.output_tokens, Some(2));
        assert!(
            matches!(observer.0.last(), Some(ProviderEvent::Completed(Completion { usage, .. })) if usage.input_tokens == Some(8) && usage.cached_input_tokens.is_none())
        );
    }

    #[tokio::test]
    async fn collector_rejects_missing_duplicate_and_failed_terminals() {
        let completion = Completion::from_legacy("done", vec![], 0, 0);
        for events in [
            vec![],
            vec![
                ProviderEvent::Completed(completion.clone()),
                ProviderEvent::Completed(completion.clone()),
            ],
            vec![ProviderEvent::Failed {
                kind: ProviderFailureKind::Incomplete,
            }],
        ] {
            assert!(
                collect_completion(&ScriptedProvider(events), request(), None)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn native_sse_preserves_partial_usage_and_stops_on_observer_write_failure() {
        let partial = HttpFixture::new(vec![Reply {
            body: "data: {\"type\":\"response.failed\",\"response\":{\"usage\":{\"input_tokens\":7,\"output_tokens\":2}}}\n\n".into(),
            status: axum::http::StatusCode::OK,
            content_type: "text/event-stream",
            session: false,
        }])
        .await;
        let provider = ResponsesProvider::new(&partial.url, "").unwrap();
        let mut observed = Events::default();
        let error = collect_completion(&provider, request(), Some(&mut observed))
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("input_tokens"));
        assert!(matches!(
            observed.0.first(),
            Some(ProviderEvent::ContextMeasured(_))
        ));
        assert!(matches!(
            observed.0.as_slice(),
            [
                _,
                ProviderEvent::Usage(Usage {
                    input_tokens: Some(7),
                    output_tokens: Some(2),
                    ..
                }),
                ProviderEvent::Failed {
                    kind: ProviderFailureKind::Failed
                }
            ]
        ));
        assert_eq!(partial.requests.lock().await.len(), 1);

        struct FailOnUsage(Vec<ProviderEvent>);
        impl ProviderSink for FailOnUsage {
            fn emit<'a>(
                &'a mut self,
                event: ProviderEvent,
            ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
                Box::pin(async move {
                    self.0.push(event.clone());
                    if matches!(event, ProviderEvent::Usage(_)) {
                        bail!("observer write failed");
                    }
                    Ok(())
                })
            }
        }
        let terminal = HttpFixture::new(vec![Reply {
            body: "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}],\"usage\":{\"input_tokens\":9,\"output_tokens\":3}}}\n\n".into(),
            status: axum::http::StatusCode::OK,
            content_type: "text/event-stream",
            session: false,
        }])
        .await;
        let provider = ResponsesProvider::new(&terminal.url, "").unwrap();
        let mut sink = FailOnUsage(vec![]);
        let error = collect_completion(&provider, request(), Some(&mut sink))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("observer write failed"));
        assert!(matches!(sink.0.last(), Some(ProviderEvent::Usage(_))));
        assert!(
            !sink
                .0
                .iter()
                .any(|event| matches!(event, ProviderEvent::Completed(_)))
        );
        assert_eq!(terminal.requests.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn api_sse_delta_reaches_observer_before_delayed_terminal() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let read = socket.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0, "client closed before sending the request");
                request.extend_from_slice(&chunk[..read]);
                let Some(headers_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = std::str::from_utf8(&request[..headers_end]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .and_then(|length| length.parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
            let delta = b"data: {\"type\":\"response.output_text.delta\",\"item_id\":\"message-1\",\"output_index\":0,\"content_index\":0,\"delta\":\"visible\"}\n\n";
            for byte in delta {
                socket.write_all(b"1\r\n").await.unwrap();
                socket.write_all(&[*byte]).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            socket.flush().await.unwrap();
            released.await.unwrap();
            let terminal = b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"output\":[{\"type\":\"message\",\"id\":\"message-1\",\"content\":[{\"type\":\"output_text\",\"text\":\"visible\"}]}]}}\n\n";
            for byte in terminal {
                socket.write_all(b"1\r\n").await.unwrap();
                socket.write_all(&[*byte]).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let provider = ResponsesProvider::new(&base, "").unwrap();
        let (sender, mut observed) = tokio::sync::mpsc::channel(8);
        let mut sink = ChannelEvents(sender);
        let mut stream = Box::pin(provider.stream(request(), &mut sink));
        let first = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                event = observed.recv() => event,
                result = &mut stream => panic!("stream settled before a delta: {result:?}"),
            }
        })
        .await
        .unwrap();
        assert!(matches!(first, Some(ProviderEvent::ContextMeasured(_))));
        let delta = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                event = observed.recv() => event,
                result = &mut stream => panic!("stream settled before a delta: {result:?}"),
            }
        })
        .await
        .unwrap();
        assert!(matches!(delta, Some(ProviderEvent::TextDelta { text, .. }) if text == "visible"));
        release.send(()).unwrap();
        stream.as_mut().await.unwrap();
        let terminal = [observed.recv().await, observed.recv().await];
        assert!(
            terminal
                .iter()
                .any(|event| matches!(event, Some(ProviderEvent::Completed(_))))
        );
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn native_function_round_trip_keeps_reasoning_and_actor_isolation() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque-private-reasoning","summary":[]},{"type":"function_call","call_id":"c1","name":"file_read","arguments":"{\"path\":\"a.txt\"}"},{"type":"function_call","call_id":"c2","name":"file_read","arguments":"{\"path\":\"b.txt\"}"}],"usage":{"input_tokens":8,"output_tokens":5}})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"other actor"}]}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"files compared"}]}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"declined"}]}]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut first = request();
        let completion = provider.complete(first.clone()).await.unwrap();
        assert_eq!(completion.calls().len(), 2);
        assert_eq!(
            (completion.input_tokens(), completion.output_tokens()),
            (8, 5)
        );
        let mut other = first.clone();
        other.actor = "other-project/part".into();
        assert_eq!(
            provider.complete(other).await.unwrap().text_projection(),
            "other actor"
        );
        first
            .messages
            .push(Message::text("assistant", "requesting file reads"));
        for (call, text) in [("c2", "second file"), ("c1", "first file")] {
            first
                .messages
                .push(Message::tool_result(call, json!(text), false));
        }
        first.current_message_count = Some(2);
        assert_eq!(
            provider
                .complete(first.clone())
                .await
                .unwrap()
                .text_projection(),
            "files compared"
        );
        first.messages.push(Message::text("user", "new turn"));
        first.current_message_count = Some(1);
        assert_eq!(
            provider.complete(first).await.unwrap().text_projection(),
            "declined"
        );
        let sent = peer.requests.lock().await;
        assert_eq!(sent[0].body["reasoning"]["effort"], "future-effort");
        assert_eq!(sent[0].body["tools"][0]["name"], "file_read");
        assert_eq!(sent[0].body["store"], false);
        assert!(
            !sent[1]
                .body
                .to_string()
                .contains("opaque-private-reasoning")
        );
        let input = sent[2].body["input"].as_array().unwrap();
        assert!(
            input
                .iter()
                .any(|item| item["encrypted_content"] == "opaque-private-reasoning")
        );
        let outputs: Vec<_> = input
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["call_id"], "c1");
        assert_eq!(outputs[0]["output"], "first file");
        assert!(
            !sent[3]
                .body
                .to_string()
                .contains("opaque-private-reasoning")
        );
    }

    #[tokio::test]
    async fn final_body_preflight_refuses_before_http_and_marks_native_chain_mandatory() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({
                "status":"completed",
                "output":[
                    {"type":"reasoning","encrypted_content":"opaque-private-reasoning","summary":[]},
                    {"type":"function_call","call_id":"c1","name":"file_read","arguments":"{}"},
                    {"type":"function_call","call_id":"c2","name":"file_read","arguments":"{}"}
                ]
            })),
            Reply::json(json!({
                "status":"completed",
                "output":[
                    {"type":"reasoning","encrypted_content":"second-private-reasoning","summary":[]},
                    {"type":"function_call","call_id":"c3","name":"file_read","arguments":"{}"}
                ]
            })),
        ])
        .await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut too_large = request();
        too_large.context_budget = Some(
            ContextBudget::resolve(
                kuru_core::Sourced::configured_assumption(100),
                None,
                Some(1),
            )
            .unwrap(),
        );
        let mut observations = Events::default();
        let error = provider
            .stream(too_large, &mut observations)
            .await
            .unwrap_err();
        let fit = error.downcast_ref::<kuru_core::ContextTooLarge>().unwrap();
        assert!(!fit.native_continuation_mandatory);
        assert!(peer.requests.lock().await.is_empty());
        assert!(matches!(
            observations.0.as_slice(),
            [ProviderEvent::ContextMeasured(_)]
        ));

        let mut continuation = request();
        assert_eq!(
            provider
                .complete(continuation.clone())
                .await
                .unwrap()
                .calls()
                .len(),
            2
        );
        continuation.messages.extend([
            Message::tool_result("c2", json!("second"), false),
            Message::tool_result("c1", json!("first"), false),
        ]);
        continuation.current_message_count = Some(2);
        assert_eq!(
            provider
                .complete(continuation.clone())
                .await
                .unwrap()
                .calls()
                .len(),
            1
        );
        assert_eq!(peer.requests.lock().await.len(), 2);
        continuation
            .messages
            .push(Message::text("user", "private-sentinel-".repeat(1_000)));
        continuation
            .messages
            .push(Message::tool_result("c3", json!("third"), false));
        continuation.current_message_count = Some(1);
        continuation.context_budget = Some(
            ContextBudget::resolve(
                kuru_core::Sourced::configured_assumption(100),
                None,
                Some(1),
            )
            .unwrap(),
        );
        let mut observations = Events::default();
        let error = provider
            .stream(continuation, &mut observations)
            .await
            .unwrap_err();
        let fit = error.downcast_ref::<kuru_core::ContextTooLarge>().unwrap();
        assert!(fit.native_continuation_mandatory);
        assert_eq!(peer.requests.lock().await.len(), 2);
        assert!(!error.to_string().contains("private-sentinel"));
        let [ProviderEvent::ContextMeasured(measurement)] = observations.0.as_slice() else {
            panic!("one context measurement must precede refusal")
        };
        assert!(measurement.final_body_bytes > 0);
        assert!(measurement.sources.iter().any(|source| source.kind
            == ContextSourceKind::NativeContinuation
            && source.mandatory));
        assert!(!format!("{measurement:?}").contains("private-sentinel"));
    }

    #[tokio::test]
    async fn offline_demo_also_measures_and_refuses_an_oversize_prompt() {
        let mut input = request();
        input.model = "demo".into();
        input.context_budget = Some(
            ContextBudget::resolve(
                kuru_core::Sourced::configured_assumption(100),
                None,
                Some(1),
            )
            .unwrap(),
        );
        let mut observations = Events::default();
        let error = DemoProvider
            .stream(input, &mut observations)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<kuru_core::ContextTooLarge>().is_some());
        assert!(matches!(
            observations.0.as_slice(),
            [ProviderEvent::ContextMeasured(_)]
        ));
    }

    #[tokio::test]
    async fn consecutive_native_calls_accept_only_the_current_receipt_batch() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[
                {"type":"reasoning","encrypted_content":"first-native-reasoning","summary":[]},
                {"type":"function_call","call_id":"c1","name":"file_read","arguments":"{}"}
            ]})),
            Reply::json(json!({"status":"completed","output":[
                {"type":"reasoning","encrypted_content":"second-native-reasoning","summary":[]},
                {"type":"function_call","call_id":"c2","name":"file_read","arguments":"{}"}
            ]})),
            Reply::json(json!({"status":"completed","output":[
                {"type":"message","content":[{"type":"output_text","text":"complete"}]}
            ]})),
        ])
        .await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();

        let first = provider.complete(input.clone()).await.unwrap();
        input.messages.push(Message {
            role: "assistant".into(),
            blocks: first.blocks,
        });
        input
            .messages
            .push(Message::tool_result("c1", json!("first receipt"), false));
        input.current_message_count = Some(1);

        let second = provider.complete(input.clone()).await.unwrap();
        input.messages.push(Message {
            role: "assistant".into(),
            blocks: second.blocks,
        });
        input
            .messages
            .push(Message::tool_result("c2", json!("second receipt"), false));
        input.current_message_count = Some(1);

        assert_eq!(
            provider.complete(input).await.unwrap().text_projection(),
            "complete"
        );
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 3);
        let outputs: Vec<_> = sent[2].body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["call_id"], "c1");
        assert_eq!(outputs[0]["output"], "first receipt");
        assert_eq!(outputs[1]["call_id"], "c2");
        assert_eq!(outputs[1]["output"], "second receipt");
        assert!(sent[2].body.to_string().contains("first-native-reasoning"));
        assert!(sent[2].body.to_string().contains("second-native-reasoning"));
    }

    #[tokio::test]
    async fn omitted_assistant_delimiter_does_not_mix_old_and_current_receipts() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"function_call","call_id":"c1","name":"file_read","arguments":"{}"}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"function_call","call_id":"c2","name":"file_read","arguments":"{}"}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();
        provider.complete(input.clone()).await.unwrap();
        input
            .messages
            .push(Message::tool_result("c1", json!("first"), false));
        input.current_message_count = Some(1);
        provider.complete(input.clone()).await.unwrap();
        // Bounded history may omit the assistant record between these results.
        input
            .messages
            .push(Message::tool_result("c2", json!("second"), false));
        input.current_message_count = Some(1);
        assert_eq!(
            provider.complete(input).await.unwrap().text_projection(),
            "done"
        );
        let sent = peer.requests.lock().await;
        let outputs = sent[2].body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["call_id"], "c1");
        assert_eq!(outputs[1]["call_id"], "c2");
    }

    #[tokio::test]
    async fn current_peer_message_between_receipts_is_sent_after_native_outputs() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"function_call","call_id":"c1","name":"file_read","arguments":"{}"},{"type":"function_call","call_id":"c2","name":"file_read","arguments":"{}"}]})),
            Reply::json(json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();
        provider.complete(input.clone()).await.unwrap();
        input.messages.extend([
            Message::tool_result("c1", json!("first"), false),
            Message::text("user", "Peer reply: check the file"),
            Message::tool_result("c2", json!("second"), false),
        ]);
        input.current_message_count = Some(3);
        assert_eq!(
            provider.complete(input).await.unwrap().text_projection(),
            "done"
        );
        let sent = peer.requests.lock().await;
        let items = sent[1].body["input"].as_array().unwrap();
        let outputs = items
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        assert_eq!(
            outputs
                .iter()
                .map(|item| item["call_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["c1", "c2"]
        );
        assert_eq!(
            items.last().unwrap()["content"],
            "Peer reply: check the file"
        );
    }

    #[tokio::test]
    async fn models_preserve_future_names_and_do_not_invent_efforts() {
        let peer = HttpFixture::new(vec![Reply::json(
            json!({"data":[{"id":"future-2099"},{"id":"special-model"}]}),
        )])
        .await;
        let models = ResponsesProvider::new(&peer.url, "")
            .unwrap()
            .models()
            .await
            .unwrap();
        assert_eq!(models[0].id, "future-2099");
        assert!(models[0].efforts.is_empty());
        assert!(models[0].metadata.prices.is_none());
    }

    #[test]
    fn official_and_custom_response_routes_have_distinct_catalog_facts() {
        let mut provider = ResponsesProvider::new("http://127.0.0.1:1", "").unwrap();
        let live = ModelInfo {
            id: "gpt-5.6-sol".into(),
            name: "Future display name".into(),
            efforts: vec!["future-effort".into()],
            default_effort: Some("future-effort".into()),
            metadata: ModelMetadata::default(),
        };
        assert!(
            provider.enrich_models(vec![live.clone()]).unwrap()[0]
                .metadata
                .prices
                .is_none()
        );
        provider.base = "https://api.openai.com/v1".into();
        let official = provider.enrich_models(vec![live]).unwrap().pop().unwrap();
        assert_eq!(official.efforts, ["future-effort"]);
        assert_eq!(
            official.metadata.context_window_tokens.unwrap().value,
            1_050_000
        );
        assert!(official.metadata.prices.is_some());
    }

    #[tokio::test]
    async fn api_key_completion_uses_its_operation_deadline() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            tokio::time::sleep(Duration::from_millis(250)).await;
        });
        let mut provider = ResponsesProvider::new(&url, "").unwrap();
        provider.completion_timeout = Duration::from_millis(80);
        let error = tokio::time::timeout(Duration::from_secs(1), provider.complete(request()))
            .await
            .unwrap()
            .unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("transport timed out")
                || diagnostic.contains("Responses request exceeded 600-second total limit"),
            "{diagnostic}"
        );
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn api_key_completion_overrides_shorter_generic_client_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            tokio::time::sleep(Duration::from_millis(150)).await;
            let body = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"later\"}]}]}}\n\n";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let mut provider = ResponsesProvider::new(&url, "").unwrap();
        provider.client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_millis(80))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        provider.completion_timeout = Duration::from_secs(2);
        let completion = tokio::time::timeout(Duration::from_secs(3), provider.complete(request()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completion.text_projection(), "later");
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn api_key_catalog_keeps_its_separate_http_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            tokio::time::sleep(Duration::from_millis(150)).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 32\r\nConnection: close\r\n\r\n{\"data\":[{\"id\":\"future-model\"}]}",
                )
                .await
                .unwrap();
        });
        let mut provider = ResponsesProvider::new(&url, "").unwrap();
        provider.completion_timeout = Duration::from_millis(80);
        let models = tokio::time::timeout(Duration::from_secs(1), provider.models())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(models[0].id, "future-model");
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn provider_factory_and_demo_remain_offline() {
        let config = Config {
            provider: "demo".into(),
            ..Default::default()
        };
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("uncreated-data");
        let demo = provider(&config, Path::new("."), &data).await.unwrap();
        assert_eq!(demo.models().await.unwrap()[0].id, "demo");
        assert!(
            demo.complete(request())
                .await
                .unwrap()
                .text_projection()
                .contains("hello")
        );
        let mut empty = request();
        empty.messages.clear();
        assert!(
            demo.complete(empty)
                .await
                .unwrap()
                .text_projection()
                .contains("Ready")
        );
        assert!(
            provider(
                &Config {
                    provider: "responses".into(),
                    ..Default::default()
                },
                Path::new("."),
                &data
            )
            .await
            .is_ok()
        );
        assert!(
            provider(
                &Config {
                    provider: "codex".into(),
                    ..Default::default()
                },
                Path::new("."),
                &data
            )
            .await
            .is_err()
        );
        assert!(
            !data.exists(),
            "provider selection created authentication state"
        );
        assert!(
            provider(
                &Config {
                    provider: "unknown".into(),
                    ..Default::default()
                },
                Path::new("."),
                &data,
            )
            .await
            .is_err()
        );
        let api =
            ResponsesProvider::new("http://localhost:1", "KURU_NONEXISTENT_AUTH_VARIABLE").unwrap();
        assert!(
            api.models()
                .await
                .unwrap_err()
                .to_string()
                .contains("authentication environment variable is not set")
        );
        let mut auto = request();
        auto.model = "auto".into();
        assert!(
            api.complete(auto)
                .await
                .unwrap_err()
                .to_string()
                .contains("explicit model")
        );
        assert!(ResponsesProvider::new("file:///tmp/key", "").is_err());
    }

    #[test]
    fn validates_native_output_and_partial_tool_handshakes() {
        for value in [
            json!({"error":{"message":"unavailable"}}),
            json!({"status":"incomplete"}),
            json!({}),
            json!({"output":[{"type":"function_call","name":"x","arguments":"{}"}]}),
            json!({"output":[{"type":"function_call","call_id":"a","name":"x","arguments":"not-json"}]}),
            json!({"output":[{"type":"function_call","call_id":"a","name":"x","arguments":"{}"},{"type":"function_call","call_id":"a","name":"x","arguments":"{}"}]}),
        ] {
            assert!(
                completion(&value, diagnostics::Operation::ResponsesCompletion).is_err(),
                "{value}"
            );
        }
        let pending = Pending {
            input: vec![],
            native_output_ranges: vec![],
            output: vec![],
            calls: vec!["a".into(), "b".into()],
        };
        let messages = vec![Message::tool_result("a", json!("ok"), false)];
        assert!(input_items(&messages, Some(&pending), Some(1)).is_err());
        assert_eq!(
            input_items(&messages, None, Some(1)).unwrap()[0]["role"],
            "user"
        );
        assert_eq!(
            input_items(&request().messages, Some(&pending), Some(1))
                .unwrap()
                .len(),
            1
        );
        let malformed = vec![Message::text("tool", "not JSON")];
        assert!(
            input_items(&malformed, Some(&pending), Some(1))
                .unwrap_err()
                .to_string()
                .contains("invalid current tool receipt")
        );
        let mixed = vec![Message {
            role: "tool".into(),
            blocks: vec![
                ContentBlock::Text {
                    text: "not a receipt".into(),
                },
                ContentBlock::ToolResult {
                    call_id: "a".into(),
                    output: json!("ok"),
                    is_error: false,
                },
            ],
        }];
        assert!(
            input_items(&mixed, Some(&pending), Some(1))
                .unwrap_err()
                .to_string()
                .contains("invalid current tool receipt")
        );
        let mismatch = vec![Message::tool_result("other", json!("no"), false)];
        let error = input_items(&mismatch, Some(&pending), Some(1)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match a pending Responses call")
        );
    }

    #[test]
    fn completion_keeps_ordered_typed_blocks_and_unknown_usage_absent() {
        let completion = completion(
            &json!({
                "status":"completed",
                "output":[
                    {"type":"message","content":[{"type":"output_text","text":"before"}]},
                    {"type":"function_call","call_id":"call-one","name":"file_read","arguments":"{\"path\":\"one\"}"},
                    {"type":"message","content":[{"type":"refusal","refusal":"after"}]}
                ],
                "usage": {}
            }),
            diagnostics::Operation::ResponsesCompletion,
        )
        .unwrap();
        assert_eq!(completion.text_projection(), "before\nafter");
        assert_eq!(completion.calls()[0].id, "call-one");
        assert_eq!(completion.usage.input_tokens, None);
        assert_eq!(completion.usage.output_tokens, None);
        assert!(matches!(
            completion.blocks.as_slice(),
            [
                ContentBlock::Text { text },
                ContentBlock::ToolUse { id, .. },
                ContentBlock::Text { .. }
            ] if text == "before" && id == "call-one"
        ));
    }

    #[tokio::test]
    async fn unsupported_typed_content_is_rejected_before_dispatch() {
        let peer = HttpFixture::new(vec![]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();
        input.messages[0].blocks = vec![ContentBlock::Image {
            media_type: "image/png".into(),
            data_base64: "AAEC".into(),
        }];
        let error = provider.complete(input).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support image or cache content blocks")
        );
        assert!(peer.requests.lock().await.is_empty());
        let mut demo_input = request();
        demo_input.messages[0].blocks = vec![ContentBlock::CacheBoundary {
            kind: "ephemeral".into(),
        }];
        let error = DemoProvider.complete(demo_input).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support image or cache content blocks")
        );

        let pending_peer = HttpFixture::new(vec![Reply::json(json!({
            "status":"completed",
            "output":[{"type":"function_call","call_id":"pending-call","name":"file_read","arguments":"{}"}]
        }))])
        .await;
        let pending_provider = ResponsesProvider::new(&pending_peer.url, "").unwrap();
        let mut continuation = request();
        pending_provider
            .complete(continuation.clone())
            .await
            .unwrap();
        continuation.messages.push(Message {
            role: "tool".into(),
            blocks: vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data_base64: "AAEC".into(),
            }],
        });
        continuation
            .messages
            .push(Message::tool_result("pending-call", json!("ok"), false));
        continuation.current_message_count = Some(2);
        let error = pending_provider.complete(continuation).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support image or cache content blocks")
        );
        assert_eq!(pending_peer.requests.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn response_envelopes_do_not_render_remote_values() {
        let peer = HttpFixture::new(vec![
            json!({"error":{"message":"http-200-message-secret","code":"unknown-code","type":"unknown-type"}}),
            json!({"status":"unexpected-status-secret","output":[]}),
        ]
        .into_iter()
        .map(Reply::json)
        .collect())
        .await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        for _ in 0..2 {
            let error = provider.complete(request()).await.unwrap_err();
            let display = format!("{error:#}");
            let debug = format!("{error:?}");
            for secret in [
                "http-200-message-secret",
                "unknown-code",
                "unknown-type",
                "unexpected-status-secret",
            ] {
                assert!(
                    !display.contains(secret) && !debug.contains(secret),
                    "remote value escaped: {display} / {debug}"
                );
            }
        }
        assert_eq!(peer.requests.lock().await.len(), 2);
    }

    #[test]
    fn provider_parser_and_environment_errors_do_not_retain_values() {
        let value = json!({"output":[{"type":"function_call","call_id":"call","name":"tool","arguments":"{arguments-secret"}]});
        let error = completion(&value, diagnostics::Operation::ResponsesCompletion).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Responses completion contained invalid function arguments"
        );
        assert!(!format!("{error:#}").contains("arguments-secret"));
        assert!(!format!("{error:?}").contains("arguments-secret"));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let error = environment_error(std::env::VarError::NotUnicode(
                std::ffi::OsString::from_vec(b"environment-value-secret\xff".to_vec()),
            ));
            assert_eq!(
                error.to_string(),
                "Responses API authentication environment variable is not valid Unicode"
            );
            assert!(!format!("{error:#}").contains("environment-value-secret"));
            assert!(!format!("{error:?}").contains("environment-value-secret"));

            let error = environment_key("configured-name-secret\0").unwrap_err();
            assert_eq!(
                error.to_string(),
                "Responses API authentication environment variable is not set"
            );
            assert!(!format!("{error:#}").contains("configured-name-secret"));
            assert!(!format!("{error:?}").contains("configured-name-secret"));
        }

        let error = environment_value(" \t".into()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Responses API authentication environment variable is empty"
        );
    }

    #[tokio::test]
    async fn responses_http_diagnostics_are_finite_and_redacted() {
        let mut model =
            Reply::json(json!({"error":{"message":"model-body-secret","code":"model_not_found"}}));
        model.status = reqwest::StatusCode::BAD_REQUEST;
        let mut quota = Reply::json(
            json!({"error":{"message":"billing-body-secret","type":"insufficient_quota"}}),
        );
        quota.status = reqwest::StatusCode::TOO_MANY_REQUESTS;
        let mut oversized = Reply::json(
            json!({"error":{"message":"oversized-body-secret","padding":"x".repeat(9 * 1024)}}),
        );
        oversized.status = reqwest::StatusCode::BAD_GATEWAY;
        let peer = HttpFixture::new(vec![model, quota, oversized]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        for expected in [
            "selected model is unavailable or access is denied (HTTP 400)",
            "quota or billing limit (HTTP 429)",
            "service failed (HTTP 502)",
        ] {
            let error = provider.complete(request()).await.unwrap_err();
            let display = format!("{error:#}");
            let debug = format!("{error:?}");
            assert!(display.contains(expected), "{display}");
            for secret in [
                "model-body-secret",
                "billing-body-secret",
                "oversized-body-secret",
            ] {
                assert!(
                    !display.contains(secret) && !debug.contains(secret),
                    "remote body escaped: {display} / {debug}"
                );
            }
        }
        assert_eq!(peer.requests.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn response_catalog_diagnostics_use_the_catalog_operation_and_redact_bodies() {
        let mut reply = Reply::json(
            json!({"error":{"message":"catalog-body-secret","code":"model_not_found"}}),
        );
        reply.status = reqwest::StatusCode::NOT_FOUND;
        let peer = HttpFixture::new(vec![reply]).await;
        let error = ResponsesProvider::new(&peer.url, "")
            .unwrap()
            .models()
            .await
            .unwrap_err();
        let display = format!("{error:#}");
        let debug = format!("{error:?}");
        assert!(
            display.contains(
                "Responses model catalog selected model is unavailable or access is denied (HTTP 404)"
            ),
            "{display}"
        );
        assert!(
            !display.contains("catalog-body-secret") && !debug.contains("catalog-body-secret"),
            "catalog body escaped: {display} / {debug}"
        );
    }

    #[tokio::test]
    async fn chunked_oversized_error_body_falls_back_without_quota_classification() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            let body = json!({"error":{"type":"insufficient_quota","message":"chunked-quota-secret","padding":"x".repeat(9 * 1024)}}).to_string();
            socket
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            socket
                .write_all(format!("{:x}\r\n", body.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.write_all(b"\r\n0\r\n\r\n").await.unwrap();
        });
        let error = ResponsesProvider::new(&url, "")
            .unwrap()
            .complete(request())
            .await
            .unwrap_err();
        let display = format!("{error:#}");
        let debug = format!("{error:?}");
        assert!(display.contains("service failed (HTTP 502)"), "{display}");
        assert!(
            !display.contains("quota or billing")
                && !display.contains("chunked-quota-secret")
                && !debug.contains("chunked-quota-secret"),
            "chunked diagnostic body escaped or was classified: {display} / {debug}"
        );
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn stalled_error_body_uses_status_fallback_inside_operation_budget() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            socket
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\n{\r\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
        });
        let provider = ResponsesProvider::new(&url, "").unwrap();
        let started = std::time::Instant::now();
        let error = provider.complete(request()).await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "diagnostic body read exceeded its bounded fallback"
        );
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("service failed (HTTP 502)"),
            "{diagnostic}"
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn dribbling_error_body_uses_one_total_diagnostic_deadline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            socket
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            for _ in 0..4 {
                if socket.write_all(b"1\r\n{\r\n").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        });
        let started = std::time::Instant::now();
        let error = ResponsesProvider::new(&url, "")
            .unwrap()
            .complete(request())
            .await
            .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_millis(2600),
            "diagnostic deadline restarted after a body chunk"
        );
        assert!(
            format!("{error:#}").contains("service failed (HTTP 502)"),
            "{error:#}"
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn enclosing_completion_deadline_preempts_diagnostic_body_read() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert_ne!(socket.read(&mut request).await.unwrap(), 0);
            socket
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\n{\r\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let mut provider = ResponsesProvider::new(&url, "").unwrap();
        provider.completion_timeout = Duration::from_millis(150);
        let started = std::time::Instant::now();
        let error = provider.complete(request()).await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "diagnostic reader extended the enclosing completion budget"
        );
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("Responses request exceeded 600-second total limit")
                || diagnostic.contains("service failed (HTTP 502)"),
            "{diagnostic}"
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn provider_transport_errors_do_not_retain_endpoint_queries() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/?configured-query-secret=untrusted",
            listener.local_addr().unwrap()
        );
        drop(listener);
        let error = ResponsesProvider::new(&endpoint, "")
            .unwrap()
            .complete(request())
            .await
            .unwrap_err();
        let display = format!("{error:#}");
        let debug = format!("{error:?}");
        assert!(display.contains("transport connection failed"), "{display}");
        assert!(
            !display.contains("configured-query-secret")
                && !debug.contains("configured-query-secret"),
            "endpoint query escaped: {display} / {debug}"
        );
    }

    #[tokio::test]
    async fn failed_round_trip_can_be_retried_without_losing_pending_output() {
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"output":[{"type":"function_call","call_id":"a","name":"file_read","arguments":"{}"}]})),
            Reply::json(json!({"error":{"message":"temporary unavailable"}})),
            Reply::json(json!({"output":[]})),
        ]).await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        let mut input = request();
        input.effort = None;
        provider.complete(input.clone()).await.unwrap();
        input
            .messages
            .push(Message::tool_result("a", json!({"answer":4}), false));
        assert!(provider.complete(input.clone()).await.is_err());
        provider.complete(input).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent[1].body, sent[2].body);
        assert_eq!(
            sent[2].body["input"].as_array().unwrap().last().unwrap()["type"],
            "function_call_output"
        );
    }

    #[tokio::test]
    async fn rejected_provider_status_retries_identical_request_within_shared_budget() {
        let mut unavailable = Reply::json(json!({"error":{"message":"retry-secret"}}));
        unavailable.status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
        let peer = HttpFixture::new(vec![
            unavailable,
            Reply::json(json!({"status":"completed","output":[]})),
        ])
        .await;
        let provider = ResponsesProvider::new(&peer.url, "").unwrap();
        provider.complete(request()).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].body, sent[1].body);
        assert_eq!(sent[0].method, sent[1].method);

        let mut first = Reply::json(json!({"error":{"message":"catalog-retry-secret"}}));
        first.status = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            first,
            Reply::json(json!({"data":[{"id":"future-model"}]})),
        ])
        .await;
        assert_eq!(
            ResponsesProvider::new(&peer.url, "")
                .unwrap()
                .models()
                .await
                .unwrap()[0]
                .id,
            "future-model"
        );
        assert_eq!(peer.requests.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn api_key_completion_and_catalog_retry_only_explicit_statuses() {
        for status in [
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let mut rejected = Reply::json(json!({"error":{"message":"remote-secret"}}));
            rejected.status = status;
            let peer = HttpFixture::new(vec![
                rejected,
                Reply::json(json!({"status":"completed","output":[]})),
            ])
            .await;
            ResponsesProvider::new(&peer.url, "")
                .unwrap()
                .complete(request())
                .await
                .unwrap();
            let sent = peer.requests.lock().await;
            assert_eq!(sent.len(), 2, "status {status}");
            assert_eq!(sent[0].body, sent[1].body);

            let mut rejected = Reply::json(json!({"error":{"message":"remote-secret"}}));
            rejected.status = status;
            let peer = HttpFixture::new(vec![
                rejected,
                Reply::json(json!({"data":[{"id":"future-model"}]})),
            ])
            .await;
            ResponsesProvider::new(&peer.url, "")
                .unwrap()
                .models()
                .await
                .unwrap();
            let sent = peer.requests.lock().await;
            assert_eq!(sent.len(), 2, "catalog status {status}");
            assert_eq!(sent[0].method, reqwest::Method::GET);
            assert_eq!(sent[1].method, reqwest::Method::GET);
        }

        for status in [
            reqwest::StatusCode::REQUEST_TIMEOUT,
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::StatusCode::GATEWAY_TIMEOUT,
        ] {
            let mut rejected = Reply::json(json!({"error":{"message":"terminal-secret"}}));
            rejected.status = status;
            let peer = HttpFixture::new(vec![rejected]).await;
            let error = ResponsesProvider::new(&peer.url, "")
                .unwrap()
                .complete(request())
                .await
                .err()
                .unwrap();
            assert_eq!(peer.requests.lock().await.len(), 1, "status {status}");
            assert!(!format!("{error:#}").contains("terminal-secret"));
        }
    }

    #[tokio::test]
    async fn retry_exhaustion_and_backoff_cancellation_never_send_late() {
        let replies = (0..3)
            .map(|_| {
                let mut reply = Reply::json(json!({"error":{"message":"bounded-secret"}}));
                reply.status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
                reply
            })
            .collect();
        let peer = HttpFixture::new(replies).await;
        let error = ResponsesProvider::new(&peer.url, "")
            .unwrap()
            .complete(request())
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Responses completion retry budget exhausted after 3 provider attempts"
        );
        assert_eq!(peer.requests.lock().await.len(), 3);
        assert!(!format!("{error:?}").contains("bounded-secret"));

        let mut unavailable = Reply::json(json!({"error":{}}));
        unavailable.status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
        let peer = HttpFixture::new(vec![
            Reply::json(json!({"status":"completed","output":[{"type":"function_call","call_id":"pending-call","name":"file_read","arguments":"{}"}]})),
            unavailable,
            Reply::json(json!({"status":"completed","output":[]})),
        ])
        .await;
        let provider = Arc::new(ResponsesProvider::new(&peer.url, "").unwrap());
        let mut continuation = request();
        provider.complete(continuation.clone()).await.unwrap();
        continuation.messages.push(Message::tool_result(
            "pending-call",
            json!("preserved"),
            false,
        ));
        continuation.current_message_count = Some(1);
        let task = {
            let provider = provider.clone();
            let continuation = continuation.clone();
            tokio::spawn(async move { provider.complete(continuation).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if peer.requests.lock().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(peer.requests.lock().await.len(), 2);
        provider.complete(continuation).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[1].body, sent[2].body);
    }

    #[tokio::test]
    async fn accepted_transport_disconnect_is_never_replayed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = accepted.clone();
        let mut server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0, "request ended before its complete body");
                bytes.extend_from_slice(&chunk[..count]);
                let text = String::from_utf8_lossy(&bytes);
                let Some(header_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text[..header_end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= header_end + 4 + length {
                    break;
                }
            }
            socket.shutdown().await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_secs(1), listener.accept())
                    .await
                    .is_err(),
                "ambiguous provider request was replayed"
            );
        });
        let error = ResponsesProvider::new(&url, "")
            .unwrap()
            .complete(request())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("transport"));
        match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                server.abort();
                let _ = server.await;
                panic!("ambiguous transport peer did not finish")
            }
        }
        assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
