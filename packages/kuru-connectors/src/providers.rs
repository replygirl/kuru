use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use kuru_core::{Completion, CompletionRequest, Config, Message, ModelInfo, ToolCall};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    auth::{AuthManager, AuthRoute, RequestCredentials},
    http,
};

mod sse;
#[cfg(test)]
mod subscription_tests;

const SUBSCRIPTION_BASE: &str = "https://chatgpt.com/backend-api/codex";
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);
// This describes the audited catalog wire contract, not Kuru's identity.
const CATALOG_COMPATIBILITY: &str = "0.154.0";

#[async_trait]
pub trait Provider: Send + Sync {
    async fn models(&self) -> Result<Vec<ModelInfo>>;
    async fn complete(&self, request: CompletionRequest) -> Result<Completion>;
}

pub async fn provider(config: &Config, cwd: &Path, data_dir: &Path) -> Result<Arc<dyn Provider>> {
    match config.provider.as_str() {
        "demo" => Ok(Arc::new(DemoProvider)),
        "codex" => {
            let manager = AuthManager::new(data_dir.to_owned(), cwd.to_owned(), None)?;
            let initial = manager.credentials(AuthRoute::Chatgpt).await?;
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

#[async_trait]
impl Provider for DemoProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "demo".into(),
            name: "Deterministic demo (offline)".into(),
            efforts: vec![],
            default_effort: None,
        }])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let latest = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.as_str())
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
        Ok(Completion {
            text,
            calls: vec![],
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

#[derive(Clone)]
struct Pending {
    input: Vec<Value>,
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
        http::endpoint(base)?;
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            auth: Authentication::Environment(key_env.into()),
            client: http::client()?,
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
                .build()?,
            completion_timeout: COMPLETION_TIMEOUT,
            actors: Mutex::new(BTreeMap::new()),
        })
    }

    fn is_subscription(&self) -> bool {
        matches!(self.auth, Authentication::Subscription { .. })
    }

    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        match &self.auth {
            Authentication::Environment(key_env) => {
                let builder = if key_env.is_empty() {
                    builder
                } else {
                    let key = std::env::var(key_env).with_context(|| {
                        format!("set {key_env} for Responses API authentication")
                    })?;
                    ensure!(!key.trim().is_empty(), "{key_env} is empty");
                    builder.bearer_auth(key)
                };
                Ok(builder.send().await?)
            }
            Authentication::Subscription { manager, initial } => {
                let credentials = manager.credentials(AuthRoute::Chatgpt).await?;
                same_session(initial, &credentials)?;
                let builder = builder
                    .header(
                        reqwest::header::USER_AGENT,
                        concat!("Kuru/", env!("CARGO_PKG_VERSION")),
                    )
                    .header("originator", "kuru");
                let first = subscription_headers(
                    builder
                        .try_clone()
                        .context("subscription request is not replayable")?,
                    &credentials,
                )?
                .send()
                .await?;
                if first.status() != reqwest::StatusCode::UNAUTHORIZED {
                    return Ok(first);
                }
                // Only a rejected HTTP response, before reading any stream, can
                // rotate and resend once. Never retry an uncertain partial turn.
                drop(first);
                let refreshed = manager.refresh_rejected(&credentials).await?;
                same_session(initial, &refreshed)?;
                Ok(subscription_headers(builder, &refreshed)?.send().await?)
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

fn text_message(message: &Message) -> Value {
    match message.role.as_str() {
        "assistant" | "system" | "developer" | "user" => {
            json!({"role":message.role,"content":message.content})
        }
        _ => {
            json!({"role":"user","content":format!("[{} record]\n{}", message.role, message.content)})
        }
    }
}

fn input_items(messages: &[Message], pending: Option<&Pending>) -> Result<Vec<Value>> {
    // Runtime tool continuations end with a tool receipt. A trailing user
    // message explicitly starts a new conversational/model phase.
    if messages
        .last()
        .is_some_and(|message| message.role == "user")
    {
        return Ok(messages.iter().map(text_message).collect());
    }
    let Some(pending) = pending else {
        return Ok(messages.iter().map(text_message).collect());
    };
    let mut found = BTreeMap::new();
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role != "tool" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&message.content)
            && let Some(id) = value["call_id"].as_str()
            && pending.calls.iter().any(|call| call == id)
        {
            found.entry(id.to_owned()).or_insert((index, value));
        }
    }
    // New non-tool user input starts a fresh protocol context. Historical tool
    // records after a restart are deliberately represented as ordinary text.
    if found.is_empty() {
        return Ok(messages.iter().map(text_message).collect());
    }
    if found.len() != pending.calls.len() {
        let latest_output = found.values().map(|(index, _)| *index).max().unwrap_or(0);
        if messages
            .iter()
            .skip(latest_output + 1)
            .any(|message| message.role == "user")
        {
            // A canceled operation can leave partial receipts. An explicit new
            // user turn starts fresh instead of trapping this actor forever.
            return Ok(messages.iter().map(text_message).collect());
        }
        bail!("missing function outputs for pending Responses calls");
    }
    let mut input = pending.input.clone();
    input.extend(pending.output.clone());
    let mut final_index = 0;
    for call in &pending.calls {
        let (index, record) = &found[call];
        final_index = final_index.max(*index);
        let output = record.get("output").context("tool result lacks output")?;
        input.push(json!({"type":"function_call_output","call_id":call,"output":output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string())}));
    }
    input.extend(messages.iter().skip(final_index + 1).map(text_message));
    Ok(input)
}

fn completion(value: &Value) -> Result<Completion> {
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        bail!(
            "Responses API error: {}",
            error["message"].as_str().unwrap_or("request failed")
        );
    }
    if let Some(status) = value["status"].as_str() {
        ensure!(
            status == "completed",
            "Responses API returned {status}; response was not completed"
        );
    }
    let output = value["output"]
        .as_array()
        .context("Responses response lacks output array")?;
    let mut text = Vec::new();
    let mut calls = Vec::new();
    let mut ids = BTreeSet::new();
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                if let Some(content) = item["content"].as_array() {
                    for part in content {
                        if let Some(value) =
                            part["text"].as_str().or_else(|| part["refusal"].as_str())
                        {
                            text.push(value.to_owned());
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
                .context("invalid function arguments JSON")?;
                calls.push(ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            _ => {}
        }
    }
    Ok(Completion {
        text: text.join("\n"),
        calls,
        input_tokens: value["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: value["usage"]["output_tokens"].as_u64().unwrap_or(0),
    })
}

#[async_trait]
impl Provider for ResponsesProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        tokio::time::timeout(crate::IO_TIMEOUT, async {
            let mut url = reqwest::Url::parse(&format!("{}/models", self.base))?;
            if self.is_subscription() {
                url.query_pairs_mut()
                    .append_pair("client_version", CATALOG_COMPATIBILITY);
            }
            let builder = self.client.get(url).timeout(crate::IO_TIMEOUT);
            let value = http::json(self.send(builder).await?).await?;
            if self.is_subscription() {
                return subscription_models(&value);
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
                    })
                })
                .collect()
        })
        .await
        .context("model catalog exceeded 60-second total limit")?
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        tokio::time::timeout(self.completion_timeout, self.complete_request(request))
            .await
            .context("Responses request exceeded 600-second total limit")?
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
            })
        })
        .collect()
}

impl ResponsesProvider {
    async fn complete_request(&self, request: CompletionRequest) -> Result<Completion> {
        ensure!(
            request.model != "auto",
            "select an explicit model for the Responses provider"
        );
        let actor = self.actor(&request.actor).await?;
        let mut pending = actor.lock().await;
        let input = input_items(&request.messages, pending.as_ref())?;
        let mut body = json!({"model":request.model,"instructions":request.instructions,"input":input,"store":false,"include":["reasoning.encrypted_content"],"tools":request.tools.iter().map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters,"strict":false})).collect::<Vec<_>>()});
        if let Some(effort) = request.effort {
            body["reasoning"] = json!({"effort":effort});
        }
        if self.is_subscription() {
            body["stream"] = json!(true);
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(true);
        }
        ensure!(
            body.to_string().len() <= crate::MAX_BYTES,
            "Responses request exceeds 2 MiB transport limit; shorten actor context"
        );
        let mut builder = self
            .client
            .post(format!("{}/responses", self.base))
            .timeout(self.completion_timeout)
            .json(&body);
        if self.is_subscription() {
            builder = builder.header(reqwest::header::ACCEPT, "text/event-stream");
        }
        let response = self.send(builder).await?;
        let value = if self.is_subscription() {
            sse::response(response, crate::IO_TIMEOUT).await?
        } else {
            http::json(response).await?
        };
        let result = completion(&value)?;
        *pending = if result.calls.is_empty() {
            None
        } else {
            Some(Pending {
                input,
                output: value["output"]
                    .as_array()
                    .context("missing output")?
                    .clone(),
                calls: result.calls.iter().map(|call| call.id.clone()).collect(),
            })
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{HttpFixture, Reply, request};

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
        assert_eq!(completion.calls.len(), 2);
        assert_eq!((completion.input_tokens, completion.output_tokens), (8, 5));
        let mut other = first.clone();
        other.actor = "other-project/part".into();
        assert_eq!(provider.complete(other).await.unwrap().text, "other actor");
        first.messages.push(Message {
            role: "assistant".into(),
            content: "requesting file reads".into(),
        });
        for (call, text) in [("c2", "second file"), ("c1", "first file")] {
            first.messages.push(Message {
                role: "tool".into(),
                content: json!({"call_id":call,"output":text}).to_string(),
            });
        }
        assert_eq!(
            provider.complete(first.clone()).await.unwrap().text,
            "files compared"
        );
        first.messages.push(Message {
            role: "user".into(),
            content: "new turn".into(),
        });
        assert_eq!(provider.complete(first).await.unwrap().text, "declined");
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
            diagnostic.contains("operation timed out")
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
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 81\r\nConnection: close\r\n\r\n{\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"text\":\"later\"}]}]}",
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
        assert_eq!(completion.text, "later");
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
                .text
                .contains("hello")
        );
        let mut empty = request();
        empty.messages.clear();
        assert!(demo.complete(empty).await.unwrap().text.contains("Ready"));
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
                .contains("set KURU_")
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
            assert!(completion(&value).is_err(), "{value}");
        }
        let pending = Pending {
            input: vec![],
            output: vec![],
            calls: vec!["a".into(), "b".into()],
        };
        let messages = vec![Message {
            role: "tool".into(),
            content: json!({"call_id":"a","output":"ok"}).to_string(),
        }];
        assert!(input_items(&messages, Some(&pending)).is_err());
        assert_eq!(input_items(&messages, None).unwrap()[0]["role"], "user");
        assert_eq!(
            input_items(&request().messages, Some(&pending))
                .unwrap()
                .len(),
            1
        );
        let malformed = vec![Message {
            role: "tool".into(),
            content: "not JSON".into(),
        }];
        assert_eq!(
            input_items(&malformed, Some(&pending)).unwrap()[0]["role"],
            "user"
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
        input.messages.push(Message {
            role: "tool".into(),
            content: json!({"call_id":"a","output":{"answer":4}}).to_string(),
        });
        assert!(provider.complete(input.clone()).await.is_err());
        provider.complete(input).await.unwrap();
        let sent = peer.requests.lock().await;
        assert_eq!(sent[1].body, sent[2].body);
        assert_eq!(
            sent[2].body["input"].as_array().unwrap().last().unwrap()["type"],
            "function_call_output"
        );
    }
}
