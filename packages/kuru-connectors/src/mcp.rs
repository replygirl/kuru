use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use futures::future::join_all;
use kuru_core::{McpConfig, PermissionSelector, ToolSpec};
use kuru_platform::fs::Directory;
#[cfg(test)]
use kuru_platform::fs::{NameRetention, Privacy};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::{
    IO_TIMEOUT, MAX_BYTES, http,
    mcp_cache::{CachedMcpTool, McpCatalogStore},
    rpc::Rpc,
    tool_output::ToolFailureKind,
};

const VERSION: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_STATIC_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_STATIC_HEADERS_BYTES: usize = 64 * 1024;

pub(crate) struct McpHosts {
    clients: BTreeMap<String, Arc<McpClient>>,
    disabled: BTreeSet<String>,
    routes: RwLock<BTreeMap<String, (String, String)>>,
    admission: Arc<Admission>,
    cache: OnceLock<Arc<McpCatalogStore>>,
}

pub(crate) struct Admission {
    closed: AtomicBool,
    #[cfg(test)]
    pause_launch: AtomicBool,
    #[cfg(test)]
    launched: Notify,
    #[cfg(test)]
    resume_launch: Notify,
    #[cfg(test)]
    fail_setup: AtomicBool,
}

impl Admission {
    pub(crate) fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            #[cfg(test)]
            pause_launch: AtomicBool::new(false),
            #[cfg(test)]
            launched: Notify::new(),
            #[cfg(test)]
            resume_launch: Notify::new(),
            #[cfg(test)]
            fail_setup: AtomicBool::new(false),
        }
    }

    pub(crate) fn enter(&self) -> Result<()> {
        ensure!(!self.is_closed(), "MCP host is closed");
        Ok(())
    }

    fn close_admission(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) async fn after_launch(&self) {
        if self.pause_launch.swap(false, Ordering::AcqRel) {
            let resume = self.resume_launch.notified();
            tokio::pin!(resume);
            self.launched.notify_waiters();
            resume.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_setup(&self) -> bool {
        self.fail_setup.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAvailability {
    Disabled,
    Live,
    Stale,
    Degraded,
}

#[derive(Debug, Serialize)]
pub struct McpStatus {
    alias: String,
    availability: McpAvailability,
    diagnostic: Option<String>,
}

impl McpStatus {
    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn available(&self) -> bool {
        self.availability == McpAvailability::Live
    }

    pub const fn availability(&self) -> McpAvailability {
        self.availability
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

pub(crate) struct McpCatalog {
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) selectors: BTreeMap<String, PermissionSelector>,
    pub(crate) statuses: Vec<McpStatus>,
}

#[derive(Debug)]
pub(crate) enum McpExecution {
    Success(Value),
    ApplicationError(Value),
}

#[derive(Debug)]
pub(crate) struct McpCallFailure {
    pub(crate) kind: ToolFailureKind,
    pub(crate) error: anyhow::Error,
}

impl McpCallFailure {
    fn route(error: anyhow::Error) -> Self {
        Self {
            kind: ToolFailureKind::McpRoute,
            error,
        }
    }

    fn call(alias: &str) -> Self {
        Self {
            kind: ToolFailureKind::McpCall,
            error: anyhow::anyhow!("configured MCP server {alias} is unavailable"),
        }
    }
}

impl McpHosts {
    #[cfg(test)]
    pub fn new(root: &Path, configs: &BTreeMap<String, McpConfig>) -> Result<Self> {
        let root = root.canonicalize().context("MCP root does not exist")?;
        let root = Arc::new(Directory::open(
            &root,
            Privacy::Inherited,
            NameRetention::Pinned,
        )?);
        Self::with_retained_root(root, configs)
    }

    pub(crate) fn with_retained_root(
        root_guard: Arc<Directory>,
        configs: &BTreeMap<String, McpConfig>,
    ) -> Result<Self> {
        root_guard.revalidate()?;
        let root = root_guard.path().to_path_buf();
        let mut clients = BTreeMap::new();
        let mut disabled = BTreeSet::new();
        let admission = Arc::new(Admission::new());
        for (name, config) in configs {
            ensure!(
                config.command.is_some() != config.url.is_some(),
                "MCP {name} needs exactly one command or URL"
            );
            ensure!(
                config.command.is_none() || config.header_env.is_empty(),
                "stdio MCP {name} cannot specify HTTP headers"
            );
            if let Some(url) = &config.url {
                http::endpoint(url)?;
            }
            if !config.enabled {
                disabled.insert(name.clone());
                continue;
            }
            clients.insert(
                name.clone(),
                Arc::new(McpClient {
                    config: config.clone(),
                    root: root.clone(),
                    root_guard: root_guard.clone(),
                    state: Mutex::new(ClientState::default()),
                    available: AtomicBool::new(false),
                    admission: admission.clone(),
                    stop: Notify::new(),
                }),
            );
        }
        Ok(Self {
            clients,
            disabled,
            routes: RwLock::new(BTreeMap::new()),
            admission,
            cache: OnceLock::new(),
        })
    }

    pub(crate) fn install_cache(&self, cache: Arc<McpCatalogStore>) -> Result<()> {
        self.cache
            .set(cache)
            .map_err(|_| anyhow::anyhow!("MCP catalog cache is already installed"))
    }

    #[cfg(test)]
    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        Ok(self.catalog().await?.tools)
    }

    pub(crate) async fn catalog(&self) -> Result<McpCatalog> {
        ensure_open(&self.admission)?;
        let discoveries = self.clients.iter().map(|(alias, client)| async move {
            let mut state = client.state.lock().await;
            ensure_open(&self.admission)?;
            let mut disable = DisableOnDrop::new(&client.available);
            let headers = match client.resolved_http_headers() {
                Ok(headers) => headers,
                Err(_) => {
                    self.routes
                        .write()
                        .await
                        .retain(|_, (owner, _)| owner != alias);
                    return Ok::<_, anyhow::Error>((
                        Vec::new(),
                        BTreeMap::new(),
                        McpStatus {
                            alias: alias.clone(),
                            availability: McpAvailability::Degraded,
                            diagnostic: Some("configured MCP static header is unavailable".into()),
                        },
                    ));
                }
            };
            let context = catalog_context(alias, &client.config, &headers)?;
            let cached = self.load_cache(alias, context).await;
            let cache_invalid = cached.is_err();
            let cached = cached.ok().flatten();
            let tools = match client.list(&mut state, headers).await {
                Ok(tools) => tools,
                Err(_) => {
                    if !state.close_attempted {
                        let _ = close_transport(&mut state).await;
                    }
                    self.routes
                        .write()
                        .await
                        .retain(|_, (owner, _)| owner != alias);
                    let (tools, selectors, availability) = match cached {
                        Some(cached) => {
                            let (tools, selectors) = project_cached(alias, cached)?;
                            (tools, selectors, McpAvailability::Stale)
                        }
                        None => (Vec::new(), BTreeMap::new(), McpAvailability::Degraded),
                    };
                    return Ok::<_, anyhow::Error>((
                        tools,
                        selectors,
                        McpStatus {
                            alias: alias.clone(),
                            availability,
                            diagnostic: state.diagnostic.clone().or_else(|| {
                                cache_invalid.then(|| {
                                    "configured MCP server and its cached catalog are unavailable"
                                        .into()
                                })
                            }),
                        },
                    ));
                }
            };
            let candidate = project_live(alias, &client.config, tools);
            match candidate {
                Ok((candidate_routes, candidate_specs, selectors, cached_tools)) => {
                    let mut routes = self.routes.write().await;
                    if candidate_routes
                        .keys()
                        .any(|name| routes.get(name).is_some_and(|(owner, _)| owner != alias))
                    {
                        routes.retain(|_, (owner, _)| owner != alias);
                        drop(routes);
                        let _ = close_transport(&mut state).await;
                        return Ok::<_, anyhow::Error>((
                            Vec::new(),
                            BTreeMap::new(),
                            McpStatus {
                                alias: alias.clone(),
                                availability: McpAvailability::Degraded,
                                diagnostic: state.diagnostic.clone(),
                            },
                        ));
                    }
                    routes.retain(|_, (owner, _)| owner != alias);
                    for (name, route) in candidate_routes {
                        routes.insert(name, route);
                    }
                    client.available.store(true, Ordering::Release);
                    disable.disarm();
                    let cache_failure =
                        self.save_cache(alias, context, cached_tools).await.is_err();
                    Ok::<_, anyhow::Error>((
                        candidate_specs,
                        selectors,
                        McpStatus {
                            alias: alias.clone(),
                            availability: McpAvailability::Live,
                            diagnostic: cache_failure
                                .then(|| "MCP catalog cache could not be updated".into()),
                        },
                    ))
                }
                Err(_) => {
                    let _ = close_transport(&mut state).await;
                    self.routes
                        .write()
                        .await
                        .retain(|_, (owner, _)| owner != alias);
                    let (tools, selectors, availability) = match cached {
                        Some(cached) => {
                            let (tools, selectors) = project_cached(alias, cached)?;
                            (tools, selectors, McpAvailability::Stale)
                        }
                        None => (Vec::new(), BTreeMap::new(), McpAvailability::Degraded),
                    };
                    Ok::<_, anyhow::Error>((
                        tools,
                        selectors,
                        McpStatus {
                            alias: alias.clone(),
                            availability,
                            diagnostic: state.diagnostic.clone(),
                        },
                    ))
                }
            }
        });
        let mut specs = Vec::new();
        let mut selectors = BTreeMap::new();
        let mut statuses = self
            .disabled
            .iter()
            .map(|alias| McpStatus {
                alias: alias.clone(),
                availability: McpAvailability::Disabled,
                diagnostic: None,
            })
            .collect::<Vec<_>>();
        for result in join_all(discoveries).await {
            let (candidate_specs, candidate_selectors, status) = result?;
            specs.extend(candidate_specs);
            for (name, selector) in candidate_selectors {
                ensure!(
                    selectors.insert(name, selector).is_none(),
                    "duplicate projected MCP tool name"
                );
            }
            statuses.push(status);
        }
        statuses.sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(McpCatalog {
            tools: specs,
            selectors,
            statuses,
        })
    }

    async fn load_cache(
        &self,
        alias: &str,
        context: [u8; 32],
    ) -> Result<Option<Vec<CachedMcpTool>>> {
        let Some(cache) = self.cache.get().cloned() else {
            return Ok(None);
        };
        let alias = alias.to_owned();
        tokio::task::spawn_blocking(move || cache.load(&alias, context))
            .await
            .context("MCP catalog cache reader stopped")?
    }

    async fn save_cache(
        &self,
        alias: &str,
        context: [u8; 32],
        tools: Vec<CachedMcpTool>,
    ) -> Result<()> {
        let Some(cache) = self.cache.get().cloned() else {
            return Ok(());
        };
        let alias = alias.to_owned();
        tokio::task::spawn_blocking(move || cache.save(&alias, context, tools))
            .await
            .context("MCP catalog cache writer stopped")?
    }

    pub(crate) async fn execute(
        &self,
        name: &str,
        arguments: Value,
    ) -> std::result::Result<McpExecution, McpCallFailure> {
        let route = self.routes.read().await.get(name).cloned();
        let (alias, original) = route.ok_or_else(|| {
            McpCallFailure::route(anyhow::anyhow!(
                "unknown tool: {name}; discover configured MCP tools first"
            ))
        })?;
        let client = self
            .clients
            .get(&alias)
            .ok_or_else(|| McpCallFailure::route(anyhow::anyhow!("MCP route no longer exists")))?;
        ensure_open(&self.admission).map_err(McpCallFailure::route)?;
        let mut state = client.state.lock().await;
        ensure_open(&self.admission).map_err(McpCallFailure::route)?;
        if !client.available.load(Ordering::Acquire) {
            return Err(McpCallFailure::call(&alias));
        }
        let mut disable = DisableOnDrop::new(&client.available);
        let result = client.call(&mut state, &original, arguments).await;
        let result = match result {
            Ok(result) => {
                disable.disarm();
                result
            }
            Err(_) => return Err(McpCallFailure::call(&alias)),
        };
        if result["isError"] == true {
            Ok(McpExecution::ApplicationError(result["content"].clone()))
        } else {
            Ok(McpExecution::Success(result))
        }
    }

    /// Resolve the provider-facing hash to its stable configured identity
    /// before permission matching. The hash itself is never an authority key.
    pub(crate) async fn selector(
        &self,
        name: &str,
    ) -> std::result::Result<PermissionSelector, McpCallFailure> {
        let route = self.routes.read().await.get(name).cloned();
        let (alias, original) = route.ok_or_else(|| {
            McpCallFailure::route(anyhow::anyhow!(
                "unknown tool: {name}; discover configured MCP tools first"
            ))
        })?;
        PermissionSelector::mcp(alias, original).map_err(McpCallFailure::route)
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.admission.close_admission();
        let mut clients = tokio::task::JoinSet::new();
        for client in self.clients.values() {
            client.stop.notify_waiters();
            let client = Arc::clone(client);
            clients.spawn(async move { client.close().await });
        }
        let mut first_error = None;
        let joined = tokio::time::timeout(IO_TIMEOUT, async {
            while let Some(result) = clients.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        first_error.get_or_insert(error);
                    }
                    Err(_) => {
                        first_error
                            .get_or_insert_with(|| anyhow::anyhow!("MCP cleanup task failed"));
                    }
                }
            }
        })
        .await;
        if joined.is_err() {
            clients.abort_all();
            return Err(anyhow::anyhow!("MCP shutdown remains unconfirmed"));
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

struct McpClient {
    config: McpConfig,
    root: PathBuf,
    root_guard: Arc<Directory>,
    state: Mutex<ClientState>,
    available: AtomicBool,
    admission: Arc<Admission>,
    stop: Notify,
}

type ProjectedCatalog = (
    BTreeMap<String, (String, String)>,
    Vec<ToolSpec>,
    BTreeMap<String, PermissionSelector>,
    Vec<CachedMcpTool>,
);

fn project_live(alias: &str, config: &McpConfig, tools: Vec<Value>) -> Result<ProjectedCatalog> {
    let mut routes = BTreeMap::new();
    let mut specs = Vec::new();
    let mut selectors = BTreeMap::new();
    let mut cached = Vec::new();
    for tool in tools {
        let original = tool["name"].as_str().context("MCP tool lacks name")?;
        if !config.admits_tool(original) {
            continue;
        }
        let selector = PermissionSelector::mcp(alias, original)?;
        ensure!(
            tool["inputSchema"].is_object(),
            "MCP tool lacks inputSchema object"
        );
        let name = projected_name(alias, original);
        ensure!(
            routes
                .insert(name.clone(), (alias.to_owned(), original.to_owned()))
                .is_none(),
            "duplicate MCP tool name for {alias}"
        );
        ensure!(
            selectors.insert(name.clone(), selector).is_none(),
            "duplicate MCP tool selector for {alias}"
        );
        let description = tool["description"]
            .as_str()
            .unwrap_or("Configured external tool")
            .to_owned();
        let parameters = tool["inputSchema"].clone();
        specs.push(ToolSpec {
            name,
            description: format!("MCP {alias}/{original}: {description}"),
            parameters: parameters.clone(),
        });
        cached.push(CachedMcpTool {
            original_name: original.to_owned(),
            description,
            parameters,
        });
    }
    Ok((routes, specs, selectors, cached))
}

fn project_cached(
    alias: &str,
    tools: Vec<CachedMcpTool>,
) -> Result<(Vec<ToolSpec>, BTreeMap<String, PermissionSelector>)> {
    let mut specs = Vec::with_capacity(tools.len());
    let mut selectors = BTreeMap::new();
    for tool in tools {
        let selector = PermissionSelector::mcp(alias, &tool.original_name)?;
        let name = projected_name(alias, &tool.original_name);
        ensure!(
            selectors.insert(name.clone(), selector).is_none(),
            "duplicate cached MCP tool name for {alias}"
        );
        specs.push(ToolSpec {
            name,
            description: format!(
                "MCP {alias}/{} (stale; unavailable until rediscovered): {}",
                tool.original_name, tool.description
            ),
            parameters: tool.parameters,
        });
    }
    Ok((specs, selectors))
}

fn projected_name(alias: &str, original: &str) -> String {
    format!(
        "mcp_{}",
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{alias}\0{original}").as_bytes()
        )
        .simple()
    )
}

fn catalog_context(alias: &str, config: &McpConfig, headers: &HeaderMap) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    digest.update(b"kuru.mcp.catalog-context.v1\0");
    hash_field(&mut digest, alias.as_bytes());
    hash_field(&mut digest, &serde_json::to_vec(config)?);
    for (name, environment) in &config.header_env {
        hash_field(&mut digest, name.as_bytes());
        hash_field(&mut digest, environment.as_bytes());
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow::anyhow!("configured MCP static header name is invalid"))?;
        let value = headers
            .get(name)
            .context("configured MCP static header value is unavailable")?;
        hash_field(&mut digest, value.as_bytes());
    }
    Ok(digest.finalize().into())
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn is_reserved_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "mcp-protocol-version"
            | "mcp-session-id"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[derive(Default)]
struct ClientState {
    transport: Option<Transport>,
    diagnostic: Option<String>,
    close_attempted: bool,
}

struct DisableOnDrop<'a> {
    available: &'a AtomicBool,
    armed: bool,
}

impl<'a> DisableOnDrop<'a> {
    fn new(available: &'a AtomicBool) -> Self {
        Self {
            available,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DisableOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.available.store(false, Ordering::Release);
        }
    }
}

impl McpClient {
    fn resolved_http_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mut total_bytes = 0_usize;
        for (name, environment) in &self.config.header_env {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("configured MCP static header name is invalid"))?;
            ensure!(
                !is_reserved_header(&name),
                "configured MCP static header is reserved for protocol ownership"
            );
            let value = static_header_value(
                environment,
                std::env::var_os(environment).with_context(|| {
                    format!("configured MCP static header environment {environment} is missing")
                })?,
            )?;
            total_bytes = total_bytes
                .checked_add(name.as_str().len())
                .and_then(|bytes| bytes.checked_add(value.as_bytes().len()))
                .context("configured MCP static headers exceed their aggregate byte limit")?;
            ensure!(
                total_bytes <= MAX_STATIC_HEADERS_BYTES,
                "configured MCP static headers exceed their aggregate byte limit"
            );
            ensure!(
                headers.insert(name, value).is_none(),
                "duplicate configured MCP static header"
            );
        }
        Ok(headers)
    }

    async fn while_open<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let stopped = self.stop.notified();
        tokio::pin!(stopped);
        ensure_open(&self.admission)?;
        tokio::pin!(operation);
        tokio::select! {
            biased;
            _ = &mut stopped => Err(anyhow::anyhow!("MCP host is closed")),
            result = &mut operation => result,
        }
    }

    async fn initialize(&self, state: &mut ClientState, headers: HeaderMap) -> Result<()> {
        state.transport = Some(if let Some(url) = &self.config.url {
            Transport::Http(HttpRpc {
                client: http::client()?,
                url: url.clone(),
                headers,
                session: None,
                version: None,
                next_id: 0,
            })
        } else {
            Transport::Stdio(Box::new(Rpc::spawn(
                self.config
                    .command
                    .as_deref()
                    .context("missing MCP command")?,
                &self.config.args,
                &self.config.env,
                &self.root,
                self.root_guard.clone(),
                self.admission.clone(),
            )?))
        });
        state.close_attempted = false;
        let initialization = async {
            let transport = state
                .transport
                .as_mut()
                .context("MCP initialization failed")?;
            self.while_open(transport.ready()).await?;
            let result = self.while_open(transport.request("initialize", json!({"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")}}))).await?;
            let version = result["protocolVersion"].as_str().context("MCP initialize lacks protocolVersion")?;
            ensure!(SUPPORTED.contains(&version), "unsupported MCP protocol version: {version}");
            ensure!(result["capabilities"]["tools"].is_object(), "MCP server did not advertise tools capability");
            if let Transport::Http(http) = &mut *transport { http.version = Some(version.into()); }
            self.while_open(transport.notify("notifications/initialized", json!({}))).await
        }.await;
        if let Err(error) = initialization {
            let _ = close_transport(state).await;
            return Err(error);
        }
        Ok(())
    }

    async fn request(&self, state: &mut ClientState, method: &str, params: Value) -> Result<Value> {
        if state.transport.is_none() {
            self.initialize(state, self.resolved_http_headers()?)
                .await?;
        }
        let result = self
            .while_open(
                state
                    .transport
                    .as_mut()
                    .context("MCP initialization failed")?
                    .request(method, params),
            )
            .await;
        if result.is_err() {
            // Never automatically retry a tool mutation. A subsequent explicit
            // call may establish a fresh session after a failed transport.
            let _ = close_transport(state).await;
        }
        result
    }

    async fn list(&self, state: &mut ClientState, headers: HeaderMap) -> Result<Vec<Value>> {
        if !self.available.load(Ordering::Acquire) {
            close_transport(state).await?;
        }
        state.diagnostic = None;
        if state
            .transport
            .as_ref()
            .is_some_and(|transport| !transport.matches_http_headers(&headers))
        {
            close_transport(state).await?;
        }
        if state.transport.is_none() {
            self.initialize(state, headers).await?;
        }
        let mut cursor = Value::Null;
        let mut seen = BTreeSet::new();
        let mut tools = Vec::new();
        loop {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let result = self.request(state, "tools/list", params).await?;
            tools.extend(
                result["tools"]
                    .as_array()
                    .context("MCP tools/list lacks tools array")?
                    .clone(),
            );
            ensure!(
                tools.len() <= 10_000,
                "MCP tool catalog exceeds 10000 tools"
            );
            cursor = result["nextCursor"].clone();
            if cursor.is_null() || cursor == "" {
                break;
            }
            ensure!(
                seen.len() < 100 && seen.insert(cursor.to_string()),
                "MCP pagination did not advance"
            );
        }
        Ok(tools)
    }

    async fn call(&self, state: &mut ClientState, name: &str, arguments: Value) -> Result<Value> {
        self.request(
            state,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
        .await
    }

    async fn close(&self) -> Result<()> {
        self.available.store(false, Ordering::Release);
        let mut state = self.state.lock().await;
        close_transport(&mut state).await
    }
}

fn static_header_value(environment: &str, value: std::ffi::OsString) -> Result<HeaderValue> {
    let value = value.into_string().map_err(|_| {
        anyhow::anyhow!("configured MCP static header environment {environment} is not Unicode")
    })?;
    ensure!(
        value.len() <= MAX_STATIC_HEADER_VALUE_BYTES,
        "configured MCP static header environment {environment} exceeds its byte limit"
    );
    HeaderValue::from_str(&value).map_err(|_| {
        anyhow::anyhow!(
            "configured MCP static header environment {environment} is not a valid header value"
        )
    })
}

async fn close_transport(state: &mut ClientState) -> Result<()> {
    let Some(transport) = state.transport.as_mut() else {
        state.close_attempted = false;
        return Ok(());
    };
    state.close_attempted = true;
    let result = transport.close().await;
    state.diagnostic = transport.stderr_diagnostic();
    result?;
    state.transport.take();
    state.close_attempted = false;
    Ok(())
}

fn ensure_open(admission: &Admission) -> Result<()> {
    ensure!(!admission.is_closed(), "MCP host is closed");
    Ok(())
}

enum Transport {
    Stdio(Box<Rpc>),
    Http(HttpRpc),
}

impl Transport {
    fn matches_http_headers(&self, headers: &HeaderMap) -> bool {
        match self {
            Self::Stdio(_) => headers.is_empty(),
            Self::Http(rpc) => rpc.headers == *headers,
        }
    }

    async fn ready(&mut self) -> Result<()> {
        match self {
            Self::Stdio(rpc) => rpc.ready().await,
            Self::Http(_) => Ok(()),
        }
    }

    fn stderr_diagnostic(&self) -> Option<String> {
        match self {
            Self::Stdio(rpc) => rpc.stderr_diagnostic(),
            Self::Http(_) => None,
        }
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match self {
            Self::Stdio(rpc) => rpc.request(method, params, IO_TIMEOUT).await,
            Self::Http(rpc) => tokio::time::timeout(IO_TIMEOUT, rpc.request(method, params))
                .await
                .context("MCP HTTP request timed out")?,
        }
    }
    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        match self {
            Self::Stdio(rpc) => {
                rpc.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
                    .await
            }
            Self::Http(rpc) => tokio::time::timeout(IO_TIMEOUT, async {
                http::body(
                    rpc.post()
                        .json(&json!({"jsonrpc":"2.0","method":method,"params":params}))
                        .send()
                        .await?,
                )
                .await?;
                Ok(())
            })
            .await
            .context("MCP HTTP notification timed out")?,
        }
    }
    async fn close(&mut self) -> Result<()> {
        match self {
            Self::Stdio(rpc) => rpc.close().await,
            Self::Http(rpc) => {
                if let Some(session) = rpc.session.clone() {
                    let mut request = rpc
                        .client
                        .delete(&rpc.url)
                        .header("Mcp-Session-Id", session);
                    for (name, value) in &rpc.headers {
                        request = request.header(name, value);
                    }
                    if let Some(version) = &rpc.version {
                        request = request.header("MCP-Protocol-Version", version);
                    }
                    let result = request.send().await?;
                    ensure!(
                        result.status().is_success()
                            || matches!(result.status().as_u16(), 404 | 405),
                        "MCP session DELETE failed: {}",
                        result.status()
                    );
                    rpc.session = None;
                }
                Ok(())
            }
        }
    }
}

struct HttpRpc {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    session: Option<String>,
    version: Option<String>,
    next_id: u64,
}

impl HttpRpc {
    fn post(&self) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream");
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(version) = &self.version {
            request = request.header("MCP-Protocol-Version", version);
        }
        request
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = json!(self.next_id);
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        ensure!(
            message.to_string().len() <= MAX_BYTES,
            "MCP request exceeds size limit"
        );
        let mut response = self.post().json(&message).send().await?;
        ensure!(
            response.status().is_success(),
            "MCP HTTP request failed: {}",
            response.status()
        );
        if let Some(session) = response.headers().get("Mcp-Session-Id") {
            self.session = Some(
                session
                    .to_str()
                    .context("invalid MCP session header")?
                    .into(),
            );
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
        {
            let mut parser = Sse::default();
            while let Some(chunk) = response.chunk().await? {
                for event in parser.push(&chunk)? {
                    if event.get("id") == Some(&id) && event.get("method").is_none() {
                        return http::rpc_result(event, &id);
                    }
                    if event.get("method").is_some() && event.get("id").is_some() {
                        let response = if event["method"] == "ping" {
                            json!({"jsonrpc":"2.0","id":event["id"],"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":event["id"],"error":{"code":-32601,"message":"Client capability not supported"}})
                        };
                        http::body(self.post().json(&response).send().await?).await?;
                    }
                }
            }
            bail!("MCP SSE stream ended before response");
        }
        http::rpc_result(http::json(response).await?, &id)
    }
}

#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: Vec<String>,
    total: usize,
}

impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.total += bytes.len();
        ensure!(
            self.total <= MAX_BYTES,
            "MCP SSE response exceeds size limit"
        );
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&line)
                .context("invalid SSE UTF-8")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(
                        serde_json::from_str(&self.data.join("\n"))
                            .context("invalid MCP SSE JSON")?,
                    );
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::test_support::drain_bounded;
    use crate::test_support::{HttpFixture, Reply, StdioFixture, Step};
    use kuru_core::{ConfigSnapshot, InvocationOverrides};
    #[cfg(unix)]
    use tokio::time::{Duration, timeout};

    fn http_config(url: &str) -> BTreeMap<String, McpConfig> {
        [(
            "test".into(),
            McpConfig {
                command: None,
                args: vec![],
                url: Some(url.into()),
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into()
    }
    fn initialized() -> Reply {
        let mut reply = Reply::rpc(json!({"protocolVersion":VERSION,"capabilities":{"tools":{}}}));
        reply.session = true;
        reply
    }
    fn tool(name: &str) -> Value {
        json!({"name":name,"description":"A fixture tool","inputSchema":{"type":"object"}})
    }

    fn cache_store(project: &Path, private: &Path) -> (Arc<Directory>, Arc<McpCatalogStore>) {
        let root =
            Arc::new(Directory::open(project, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project,
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        let store = Arc::new(
            McpCatalogStore::new(private, root.clone(), snapshot.manifest().full_digest()).unwrap(),
        );
        (root, store)
    }

    #[tokio::test]
    async fn disabled_and_filtered_aliases_never_publish_routes_or_omitted_metadata() {
        let disabled = StdioFixture::new([Step::Read, Step::Eof]);
        let disabled_http = HttpFixture::new(Vec::new()).await;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[
                tool("read_public"),
                tool("read_secret"),
                tool("write"),
                {"name":"ignored_without_schema"},
                tool(&"x".repeat(257))
            ]})),
            Reply::json(json!({})),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = BTreeMap::from([
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    command: Some(disabled.command().into()),
                    ..Default::default()
                },
            ),
            (
                "disabled-http".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled_http.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "http".into(),
                McpConfig {
                    url: Some(peer.url.clone()),
                    allow_tools: vec!["read_*".into()],
                    deny_tools: vec!["*_secret".into()],
                    ..Default::default()
                },
            ),
        ]);
        let (root_guard, cache) = cache_store(root.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root_guard, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert_eq!(catalog.statuses.len(), 3);
        assert_eq!(
            catalog.statuses[0].availability(),
            McpAvailability::Disabled
        );
        assert_eq!(
            catalog.statuses[1].availability(),
            McpAvailability::Disabled
        );
        assert_eq!(catalog.statuses[2].availability(), McpAvailability::Live);
        assert_eq!(catalog.tools.len(), 1);
        assert!(catalog.tools[0].description.contains("read_public"));
        assert!(!catalog.tools[0].description.contains("secret"));
        assert!(disabled.conversations().is_empty());
        assert!(disabled_http.requests.lock().await.is_empty());
        assert!(
            hosts
                .selector(&projected_name("http", "read_secret"))
                .await
                .is_err()
        );
        assert!(
            hosts
                .selector(&projected_name("http", "write"))
                .await
                .is_err()
        );
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn offline_restart_projects_cache_as_stale_without_an_executable_route() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let live = hosts.catalog().await.unwrap();
        assert_eq!(live.statuses[0].availability(), McpAvailability::Live);
        assert_eq!(live.tools.len(), 1);
        hosts.shutdown().await.unwrap();
        drop(hosts);
        drop(peer);

        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let restarted = McpHosts::with_retained_root(root, &config).unwrap();
        restarted.install_cache(cache).unwrap();
        let stale = restarted.catalog().await.unwrap();
        assert_eq!(stale.statuses[0].availability(), McpAvailability::Stale);
        assert_eq!(stale.tools.len(), 1);
        let name = projected_name("test", "read");
        assert!(stale.tools[0].description.contains("stale"));
        assert!(restarted.selector(&name).await.is_err());
        assert!(restarted.execute(&name, json!({})).await.is_err());
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn healthy_rediscovery_repairs_a_corrupt_cache_without_losing_its_route() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        assert_eq!(
            hosts.catalog().await.unwrap().statuses[0].availability(),
            McpAvailability::Live
        );
        let catalog_directory = std::fs::read_dir(private.path().join("data"))
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                path.is_dir().then_some(path)
            })
            .unwrap();
        let record = std::fs::read_dir(catalog_directory)
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().and_then(|value| value.to_str()) == Some("json")).then_some(path)
            })
            .unwrap();
        std::fs::write(&record, b"{").unwrap();

        let repaired = hosts.catalog().await.unwrap();
        assert_eq!(repaired.statuses[0].availability(), McpAvailability::Live);
        assert!(repaired.statuses[0].diagnostic().is_none());
        assert!(
            hosts
                .selector(&projected_name("test", "read"))
                .await
                .is_ok()
        );
        let repaired: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        assert_eq!(repaired["schema"], 1);
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn cache_write_failure_keeps_live_route_and_reports_safe_diagnostic() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"done"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let project = tempfile::tempdir().unwrap();
        let private_parent = tempfile::tempdir().unwrap();
        let private = private_parent.path().join("not-a-directory");
        let config = http_config(&peer.url);
        let (root, cache) = cache_store(project.path(), &private);
        std::fs::write(&private, b"fixture").unwrap();
        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert_eq!(catalog.statuses[0].availability(), McpAvailability::Live);
        assert_eq!(
            catalog.statuses[0].diagnostic(),
            Some("MCP catalog cache could not be updated")
        );
        let name = projected_name("test", "read");
        assert!(matches!(
            hosts.execute(&name, json!({})).await.unwrap(),
            McpExecution::Success(_)
        ));
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn mixed_alias_failures_preserve_live_and_stale_catalogs_independently() {
        let live_peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("live-read")]})),
            Reply::json(json!({})),
        ])
        .await;
        let disabled_peer = HttpFixture::new(Vec::new()).await;
        let stale_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;
        let failed_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;
        let corrupt_peer = HttpFixture::new(vec![Reply::json(json!({}))]).await;

        let config = BTreeMap::from([
            (
                "corrupt".into(),
                McpConfig {
                    url: Some(corrupt_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    url: Some(disabled_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "failed".into(),
                McpConfig {
                    url: Some(failed_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "live".into(),
                McpConfig {
                    url: Some(live_peer.url.clone()),
                    ..Default::default()
                },
            ),
            (
                "stale".into(),
                McpConfig {
                    url: Some(stale_peer.url.clone()),
                    ..Default::default()
                },
            ),
        ]);
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let (root, cache) = cache_store(project.path(), &private.path().join("data"));
        let stale_context = catalog_context("stale", &config["stale"], &HeaderMap::new()).unwrap();
        cache
            .save(
                "stale",
                stale_context,
                vec![CachedMcpTool {
                    original_name: "cached-read".into(),
                    description: "retained metadata".into(),
                    parameters: json!({"type":"object"}),
                }],
            )
            .unwrap();
        cache
            .save(
                "corrupt",
                [0xff; 32],
                vec![CachedMcpTool {
                    original_name: "must-not-project".into(),
                    description: "wrong context".into(),
                    parameters: json!({"type":"object"}),
                }],
            )
            .unwrap();

        let hosts = McpHosts::with_retained_root(root, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        let states = catalog
            .statuses
            .iter()
            .map(|status| (status.alias(), status.availability()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(states["corrupt"], McpAvailability::Degraded);
        assert_eq!(states["disabled"], McpAvailability::Disabled);
        assert_eq!(states["failed"], McpAvailability::Degraded);
        assert_eq!(states["live"], McpAvailability::Live);
        assert_eq!(states["stale"], McpAvailability::Stale);
        assert_eq!(catalog.tools.len(), 2);
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.description.contains("live-read"))
        );
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.description.contains("stale"))
        );
        assert!(
            hosts
                .selector(&projected_name("live", "live-read"))
                .await
                .is_ok()
        );
        assert!(
            hosts
                .selector(&projected_name("stale", "cached-read"))
                .await
                .is_err()
        );
        assert!(disabled_peer.requests.lock().await.is_empty());
        assert_eq!(stale_peer.requests.lock().await.len(), 1);
        assert_eq!(failed_peer.requests.lock().await.len(), 1);
        assert_eq!(corrupt_peer.requests.lock().await.len(), 1);
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn static_header_environment_reference_applies_to_the_whole_http_session() {
        let expected = std::env::var("PATH").expect("test runner PATH is required");
        HeaderValue::from_str(&expected).expect("test runner PATH must be a valid HTTP value");
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("read")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"done"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let config = BTreeMap::from([(
            "http".into(),
            McpConfig {
                url: Some(peer.url.clone()),
                header_env: BTreeMap::from([("X-Kuru-Test".into(), "PATH".into())]),
                ..Default::default()
            },
        )]);
        let (root_guard, cache) = cache_store(root.path(), &private.path().join("data"));
        let hosts = McpHosts::with_retained_root(root_guard, &config).unwrap();
        hosts.install_cache(cache).unwrap();
        let name = hosts.catalog().await.unwrap().tools[0].name.clone();
        assert!(matches!(
            hosts.execute(&name, json!({})).await.unwrap(),
            McpExecution::Success(_)
        ));
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 5);
        for request in requests.iter() {
            assert_eq!(
                request
                    .headers
                    .get("x-kuru-test")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        assert!(requests[0].headers.get("mcp-protocol-version").is_none());
        assert!(
            requests[1..]
                .iter()
                .all(|request| request.headers.get("mcp-protocol-version").is_some())
        );
        let cached = std::fs::read_dir(private.path().join("data"))
            .unwrap()
            .flat_map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap())
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .map(std::fs::read)
            .unwrap()
            .unwrap();
        assert!(
            !cached
                .windows(expected.len())
                .any(|window| window == expected.as_bytes()),
            "resolved header values must not enter catalog records"
        );
    }

    #[tokio::test]
    async fn changed_http_headers_close_the_captured_session_before_rediscovery() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("first")]})),
            Reply::json(json!({})),
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("second")]})),
            Reply::json(json!({})),
        ])
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let root = Arc::new(
            Directory::open(workspace.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let client = McpClient {
            config: http_config(&peer.url).remove("test").unwrap(),
            root: root.path().to_path_buf(),
            root_guard: root,
            state: Mutex::new(ClientState::default()),
            available: AtomicBool::new(true),
            admission: Arc::new(Admission::new()),
            stop: Notify::new(),
        };
        let mut first = HeaderMap::new();
        first.insert("x-kuru-test", HeaderValue::from_static("first"));
        let mut second = HeaderMap::new();
        second.insert("x-kuru-test", HeaderValue::from_static("second"));

        let mut state = client.state.lock().await;
        assert_eq!(
            client.list(&mut state, first).await.unwrap()[0]["name"],
            "first"
        );
        assert_eq!(
            client.list(&mut state, second).await.unwrap()[0]["name"],
            "second"
        );
        drop(state);
        client.close().await.unwrap();

        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 8);
        for request in &requests[..4] {
            assert_eq!(request.headers["x-kuru-test"], "first");
        }
        for request in &requests[4..] {
            assert_eq!(request.headers["x-kuru-test"], "second");
        }
    }

    #[tokio::test]
    async fn missing_static_header_reference_degrades_without_dispatch_or_value_diagnostic() {
        let peer = HttpFixture::new(Vec::new()).await;
        let root = tempfile::tempdir().unwrap();
        let environment = "KURU_TEST_MCP_HEADER_ENVIRONMENT_MUST_REMAIN_ABSENT_6F9320";
        assert!(std::env::var_os(environment).is_none());
        let config = BTreeMap::from([(
            "http".into(),
            McpConfig {
                url: Some(peer.url.clone()),
                header_env: BTreeMap::from([("Authorization".into(), environment.into())]),
                ..Default::default()
            },
        )]);
        let hosts = McpHosts::new(root.path(), &config).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert_eq!(
            catalog.statuses[0].availability(),
            McpAvailability::Degraded
        );
        assert_eq!(
            catalog.statuses[0].diagnostic(),
            Some("configured MCP static header is unavailable")
        );
        assert!(peer.requests.lock().await.is_empty());
        hosts.shutdown().await.unwrap();
    }

    #[test]
    fn static_header_values_are_bounded_and_value_free_on_rejection() {
        let secret = "recognizable-static-header-secret";
        let invalid = static_header_value("MCP_AUTH", format!("{secret}\n").into())
            .expect_err("a newline-bearing header value was accepted")
            .to_string();
        assert!(!invalid.contains(secret));

        let oversized_value = format!("{secret}{}", "x".repeat(MAX_STATIC_HEADER_VALUE_BYTES));
        let oversized = static_header_value("MCP_AUTH", oversized_value.into())
            .expect_err("an oversized header value was accepted")
            .to_string();
        assert!(!oversized.contains(secret));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let non_unicode = static_header_value(
                "MCP_AUTH",
                std::ffi::OsString::from_vec(vec![b's', b'e', b'c', b'r', b'e', b't', 0xff]),
            )
            .expect_err("a non-Unicode header value was accepted")
            .to_string();
            assert!(!non_unicode.contains("secret"));
        }
    }

    #[tokio::test]
    async fn http_session_paginated_discovery_call_and_shutdown() {
        let peer = HttpFixture::new(vec![initialized(),Reply::json(json!({})),Reply::rpc(json!({"tools":[tool("file_read")],"nextCursor":"p2"})),Reply::sse(json!({"tools":[tool("second")]})),Reply::sse(json!({"content":[{"type":"text","text":"called"}],"structuredContent":{"value":42}})),Reply::json(json!({}))]).await;
        let root = tempfile::tempdir().unwrap();
        let hosts = McpHosts::new(root.path(), &http_config(&peer.url)).unwrap();
        let specs = hosts.specs().await.unwrap();
        assert_eq!(specs.len(), 2);
        assert_ne!(specs[0].name, "file_read");
        assert!(specs[0].name.len() < 64);
        let McpExecution::Success(value) = hosts
            .execute(&specs[0].name, json!({"path":"a"}))
            .await
            .unwrap()
        else {
            panic!("expected an MCP success");
        };
        assert_eq!(value["structuredContent"]["value"], 42);
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests[0].body["params"]["protocolVersion"], VERSION);
        assert_eq!(requests[1].body["method"], "notifications/initialized");
        assert_eq!(requests[1].headers["mcp-session-id"], "fixture-session");
        assert_eq!(requests[2].headers["mcp-protocol-version"], VERSION);
        assert_eq!(requests[3].body["params"]["cursor"], "p2");
        assert_eq!(requests[4].body["params"]["name"], "file_read");
        assert_eq!(requests[5].method, axum::http::Method::DELETE);
    }

    #[tokio::test]
    async fn stdio_discovery_preserves_server_state_and_sequential_concurrency() {
        let counter = json!({"tools":[{"name":"counter","inputSchema":{"type":"object"}}]});
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":counter})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"1"}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"2"}]}}),
            ),
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":5,"result":counter})),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let config = [(
            "stdio".into(),
            McpConfig {
                command: Some(script.command().into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        let hosts = McpHosts::new(root.path(), &config).unwrap();
        let specs = hosts.specs().await.unwrap();
        let (one, two) = tokio::join!(
            hosts.execute(&specs[0].name, json!({})),
            hosts.execute(&specs[0].name, json!({}))
        );
        let McpExecution::Success(one) = one.unwrap() else {
            panic!("expected first MCP success");
        };
        let McpExecution::Success(two) = two.unwrap() else {
            panic!("expected second MCP success");
        };
        assert_eq!(one["content"][0]["text"], "1");
        assert_eq!(two["content"][0]["text"], "2");
        assert_eq!(hosts.specs().await.unwrap()[0].name, specs[0].name);
        hosts.shutdown().await.unwrap();
        script.assert_completed(1);
        let requests = script.conversations().remove(0);
        let methods: Vec<_> = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call",
                "tools/call",
                "tools/list"
            ]
        );
        assert_eq!(requests[3]["params"]["name"], "counter");
        assert_eq!(requests[4]["params"]["name"], "counter");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_stdio_mcp_keeps_inherited_environment_and_configured_override() {
        const CHILD: &str = "KURU_MCP_ENVIRONMENT_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let script = StdioFixture::new([
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
                ),
                Step::Read,
                Step::Read,
                Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
                Step::Read,
                Step::Eof,
            ]);
            let root = tempfile::tempdir().unwrap();
            let config = [(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    env: BTreeMap::from([(
                        "KURU_MCP_CONFIGURED_SENTINEL".into(),
                        "configured".into(),
                    )]),
                    ..Default::default()
                },
            )]
            .into();
            let hosts = McpHosts::new(root.path(), &config).unwrap();
            let specs = hosts.specs().await;
            let shutdown = hosts.shutdown().await;
            assert!(specs.unwrap().is_empty());
            shutdown.unwrap();
            assert_eq!(
                script.environment_observations(),
                vec!["inherited=true\noverride=true\n"]
            );
            return;
        }
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        let path = std::env::var_os("PATH").expect("test runner PATH is required for rustc");
        child
            .arg("--exact")
            .arg("mcp::tests::isolated_stdio_mcp_keeps_inherited_environment_and_configured_override")
            .arg("--nocapture")
            .env_clear()
            // The real stdio peer is compiled in the isolated child. Preserve
            // only its executable-discovery input; the observed MCP values are
            // controlled fake sentinels below, never dumped by the fixture.
            .env("PATH", path)
            .env("KURU_MCP_INHERITED_SENTINEL", "inherited")
            .env("KURU_MCP_CONFIGURED_SENTINEL", "parent-value")
            .env(CHILD, "1")
            .kill_on_drop(true);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        let mut child = child.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let result = timeout(Duration::from_secs(15), async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                drain_bounded(&mut stdout, &mut out),
                drain_bounded(&mut stderr, &mut err),
                child.wait(),
            );
            if stdout_truncated? || stderr_truncated? {
                return Err(std::io::Error::other(
                    "isolated MCP fixture output exceeds 2 MiB",
                ));
            }
            Ok::<_, std::io::Error>((status?, out, err))
        })
        .await;
        let (status, out, err) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                panic!(
                    "isolated MCP environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}"
                );
            }
            Err(_) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                panic!(
                    "isolated MCP environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}"
                );
            }
        };
        assert!(
            status.success(),
            "isolated MCP environment fixture failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_root_replacement_blocks_stdio_discovery_before_child_spawn() {
        let script = StdioFixture::new([Step::Read, Step::Eof]);
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let retained =
            Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Pinned).unwrap());
        let config = [(
            "stdio".into(),
            McpConfig {
                command: Some(script.command().into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        let hosts = McpHosts::with_retained_root(retained, &config).unwrap();
        std::fs::rename(&root, parent.path().join("replaced")).unwrap();
        std::fs::create_dir(&root).unwrap();

        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        assert!(script.conversations().is_empty());
    }

    #[tokio::test]
    async fn discovery_rejects_incompatible_version_duplicates_and_stalled_pagination() {
        for (reply, expected) in [
            (
                json!({"protocolVersion":"2099-01-01","capabilities":{"tools":{}}}),
                "unsupported",
            ),
            (
                json!({"protocolVersion":VERSION,"capabilities":{}}),
                "advertise",
            ),
            (json!({}), "protocolVersion"),
        ] {
            let peer = HttpFixture::new(vec![Reply::rpc(reply)]).await;
            let workspace = tempfile::tempdir().unwrap();
            let root = Arc::new(
                Directory::open(workspace.path(), Privacy::Inherited, NameRetention::Pinned)
                    .unwrap(),
            );
            let client = McpClient {
                config: http_config(&peer.url).remove("test").unwrap(),
                root: root.path().to_path_buf(),
                root_guard: root,
                state: Mutex::new(ClientState::default()),
                available: AtomicBool::new(false),
                admission: Arc::new(Admission::new()),
                stop: Notify::new(),
            };
            let mut state = client.state.lock().await;
            assert!(
                client
                    .list(&mut state, HeaderMap::new())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("a"),tool("a")]})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[],"nextCursor":"same"})),
            Reply::rpc(json!({"tools":[],"nextCursor":"same"})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();

        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":(0..=10_000).map(|index| tool(&format!("tool-{index}"))).collect::<Vec<_>>()})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn tool_application_error_retains_the_session_while_protocol_failure_is_typed() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("a")]})),
            Reply::rpc(json!({"isError":true,"content":[{"type":"text","text":"denied"}]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"usable"}]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"server failed"}})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = &hosts.specs().await.unwrap()[0].name;
        let McpExecution::ApplicationError(content) = hosts.execute(name, json!({})).await.unwrap()
        else {
            panic!("expected MCP application error");
        };
        assert_eq!(content[0]["text"], "denied");
        let McpExecution::Success(success) = hosts.execute(name, json!({})).await.unwrap() else {
            panic!("expected MCP success after application error");
        };
        assert_eq!(success["content"][0]["text"], "usable");
        let failure = hosts.execute(name, json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert_eq!(
            failure.error.to_string(),
            "configured MCP server test is unavailable"
        );
        assert_eq!(peer.requests.lock().await.len(), 7);
        hosts.shutdown().await.unwrap();
        let failure = hosts.execute("unknown", json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpRoute);
        let invalid = [(
            "bad".into(),
            McpConfig {
                command: None,
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        )]
        .into();
        assert!(McpHosts::new(Path::new("."), &invalid).is_err());
    }

    #[tokio::test]
    async fn failed_call_retains_unconfirmed_http_close_without_replay() {
        let mut rejected = Reply::json(json!({}));
        rejected.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"lost"}})),
            rejected,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = hosts.specs().await.unwrap()[0].name.clone();
        let failure = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert_eq!(peer.requests.lock().await.len(), 5);

        let disabled = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(disabled.kind, ToolFailureKind::McpCall);
        assert_eq!(peer.requests.lock().await.len(), 5);
        assert!(
            hosts.clients["test"].state.lock().await.transport.is_some(),
            "failed session cleanup must remain observable"
        );

        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[3].body["method"], "tools/call");
        assert_eq!(requests[4].method, axum::http::Method::DELETE);
        assert_eq!(requests[5].method, axum::http::Method::DELETE);
    }

    #[tokio::test]
    async fn explicit_catalog_recovers_a_failed_alias_without_replaying_its_call() {
        let recovering = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"lost"}})),
            Reply::json(json!({})),
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("mutate")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"recovered"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let steady = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("steady")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"before"}]})),
            Reply::rpc(json!({"tools":[tool("steady")]})),
            Reply::rpc(json!({"content":[{"type":"text","text":"after"}]})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([
                (
                    "recovering".into(),
                    McpConfig {
                        url: Some(recovering.url.clone()),
                        ..Default::default()
                    },
                ),
                (
                    "steady".into(),
                    McpConfig {
                        url: Some(steady.url.clone()),
                        ..Default::default()
                    },
                ),
            ]),
        )
        .unwrap();
        let initial = hosts.catalog().await.unwrap();
        let recovering_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.starts_with("MCP recovering/"))
            .unwrap()
            .name
            .clone();
        let steady_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.starts_with("MCP steady/"))
            .unwrap()
            .name
            .clone();
        let McpExecution::Success(before) = hosts.execute(&steady_name, json!({})).await.unwrap()
        else {
            panic!("expected independent alias success before recovery");
        };
        assert_eq!(before["content"][0]["text"], "before");
        assert!(hosts.execute(&recovering_name, json!({})).await.is_err());
        assert_eq!(recovering.requests.lock().await.len(), 5);
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.statuses.iter().all(McpStatus::available));
        assert!(
            catalog
                .tools
                .iter()
                .any(|tool| tool.name == recovering_name)
        );
        assert!(catalog.tools.iter().any(|tool| tool.name == steady_name));
        assert_eq!(recovering.requests.lock().await.len(), 8);
        let McpExecution::Success(result) =
            hosts.execute(&recovering_name, json!({})).await.unwrap()
        else {
            panic!("expected recovered call success");
        };
        assert_eq!(result["content"][0]["text"], "recovered");
        let McpExecution::Success(after) = hosts.execute(&steady_name, json!({})).await.unwrap()
        else {
            panic!("expected independent alias success after recovery");
        };
        assert_eq!(after["content"][0]["text"], "after");
        hosts.shutdown().await.unwrap();
        let requests = recovering.requests.lock().await;
        assert_eq!(requests.len(), 10);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .count(),
            2
        );
        let steady_requests = steady.requests.lock().await;
        assert_eq!(
            steady_requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .count(),
            2
        );
        assert_eq!(steady_requests[3].body["params"]["name"], "steady");
        assert_eq!(steady_requests[5].body["params"]["name"], "steady");
    }

    #[tokio::test]
    async fn received_stdio_mutation_disconnects_once_and_disables_later_calls() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("mutate")]}})),
            Step::Read,
        ]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        let name = hosts.specs().await.unwrap()[0].name.clone();

        let failure = hosts.execute(&name, json!({"write":"once"})).await;
        assert!(failure.is_err());
        script.assert_completed(1);
        let requests = script.conversations();
        assert_eq!(requests[0].len(), 4);
        assert_eq!(requests[0][3]["method"], "tools/call");
        assert_eq!(requests[0][3]["params"]["name"], "mutate");
        assert_eq!(requests[0][3]["params"]["arguments"]["write"], "once");

        for _ in 0..2 {
            let disabled = hosts.execute(&name, json!({"write":"again"})).await;
            assert_eq!(disabled.unwrap_err().kind, ToolFailureKind::McpCall);
        }
        assert_eq!(script.conversations()[0].len(), 4);
        hosts.shutdown().await.unwrap();
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn repeated_catalog_does_not_replace_unconfirmed_stdio_cleanup() {
        let replacement = StdioFixture::new([Step::Read]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(replacement.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        hosts.clients["stdio"].state.lock().await.transport =
            Some(Transport::Stdio(Box::new(Rpc::unconfirmed_for_test())));

        for _ in 0..2 {
            let catalog = hosts.catalog().await.unwrap();
            assert!(catalog.tools.is_empty());
            assert!(!catalog.statuses[0].available());
            assert!(
                hosts.clients["stdio"]
                    .state
                    .lock()
                    .await
                    .transport
                    .is_some()
            );
            assert!(replacement.conversations().is_empty());
        }
        assert!(hosts.shutdown().await.is_err());
        assert!(replacement.conversations().is_empty());
    }

    #[tokio::test]
    async fn cancelled_stdio_call_is_never_dispatched_twice() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("mutate")]}})),
            Step::Read,
            Step::Sleep(5_000),
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let name = hosts.specs().await.unwrap()[0].name.clone();

        let held = hosts.clients["stdio"].state.lock().await;
        let before_hosts = hosts.clone();
        let before_name = name.clone();
        let before =
            tokio::spawn(async move { before_hosts.execute(&before_name, json!({})).await });
        tokio::task::yield_now().await;
        before.abort();
        assert!(before.await.unwrap_err().is_cancelled());
        drop(held);
        assert_eq!(script.conversations()[0].len(), 3);

        let after_hosts = hosts.clone();
        let after_name = name.clone();
        let after = tokio::spawn(async move { after_hosts.execute(&after_name, json!({})).await });
        script.wait_for_requests(4).await;
        after.abort();
        assert!(after.await.unwrap_err().is_cancelled());
        let disabled = hosts.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(disabled.kind, ToolFailureKind::McpCall);
        assert_eq!(script.conversations()[0].len(), 4);
        hosts.shutdown().await.unwrap();
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn catalog_keeps_healthy_http_alias_when_stdio_alias_fails_in_either_order() {
        for failed_first in [true, false] {
            let healthy = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("usable")]})),
                Reply::json(json!({})),
            ])
            .await;
            let failed = StdioFixture::new([Step::Read, Step::Raw("not JSON"), Step::Read]);
            let (failed_alias, healthy_alias) = if failed_first {
                ("a_failed", "z_healthy")
            } else {
                ("z_failed", "a_healthy")
            };
            let config = BTreeMap::from([
                (
                    failed_alias.into(),
                    McpConfig {
                        command: Some(failed.command().into()),
                        ..Default::default()
                    },
                ),
                (
                    healthy_alias.into(),
                    McpConfig {
                        url: Some(healthy.url.clone()),
                        ..Default::default()
                    },
                ),
            ]);
            let hosts = McpHosts::new(Path::new("."), &config).unwrap();
            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1);
            assert_eq!(catalog.statuses.len(), 2);
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == healthy_alias && status.available())
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == failed_alias && !status.available())
            );
            hosts.shutdown().await.unwrap();
            assert_eq!(
                failed.conversations(),
                vec![vec![json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": VERSION,
                        "capabilities": {},
                        "clientInfo": {
                            "name": "kuru",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    },
                })]],
            );
        }
    }

    #[tokio::test]
    async fn catalog_keeps_healthy_stdio_alias_when_http_alias_fails_in_either_order() {
        for failed_first in [true, false] {
            let healthy = StdioFixture::new([
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
                ),
                Step::Read,
                Step::Read,
                Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("usable")]}})),
                Step::Read,
                Step::Write(
                    json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"used"}]}}),
                ),
                Step::Eof,
            ]);
            let mut rejected = Reply::json(json!({}));
            rejected.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
            let failed = HttpFixture::new(vec![rejected]).await;
            let (failed_alias, healthy_alias) = if failed_first {
                ("a_failed", "z_healthy")
            } else {
                ("z_failed", "a_healthy")
            };
            let config = BTreeMap::from([
                (
                    failed_alias.into(),
                    McpConfig {
                        url: Some(failed.url.clone()),
                        ..Default::default()
                    },
                ),
                (
                    healthy_alias.into(),
                    McpConfig {
                        command: Some(healthy.command().into()),
                        ..Default::default()
                    },
                ),
            ]);
            let hosts = McpHosts::new(Path::new("."), &config).unwrap();
            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1);
            assert_eq!(catalog.statuses.len(), 2);
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == healthy_alias && status.available())
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| status.alias() == failed_alias && !status.available())
            );
            let McpExecution::Success(result) = hosts
                .execute(&catalog.tools[0].name, json!({}))
                .await
                .unwrap()
            else {
                panic!("expected healthy stdio alias to remain callable");
            };
            assert_eq!(result["content"][0]["text"], "used");
            hosts.shutdown().await.unwrap();
            healthy.assert_completed(1);
            assert_eq!(failed.requests.lock().await.len(), 1);
        }
    }

    #[tokio::test]
    async fn failed_later_catalog_page_never_publishes_partial_routes() {
        for failure in ["malformed", "repeated", "oversized"] {
            let second_page = match failure {
                "malformed" => Reply::rpc(json!({})),
                "repeated" => Reply::rpc(json!({"tools":[],"nextCursor":"p2"})),
                "oversized" => Reply::rpc(json!({
                    "tools": (0..10_000)
                        .map(|index| tool(&format!("extra-{index}")))
                        .collect::<Vec<_>>()
                })),
                _ => unreachable!(),
            };
            let failed = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("partial")],"nextCursor":"p2"})),
                second_page,
                Reply::json(json!({})),
            ])
            .await;
            let healthy = HttpFixture::new(vec![
                initialized(),
                Reply::json(json!({})),
                Reply::rpc(json!({"tools":[tool("usable")]})),
                Reply::json(json!({})),
            ])
            .await;
            let hosts = McpHosts::new(
                Path::new("."),
                &BTreeMap::from([
                    (
                        "failed".into(),
                        McpConfig {
                            url: Some(failed.url.clone()),
                            ..Default::default()
                        },
                    ),
                    (
                        "healthy".into(),
                        McpConfig {
                            url: Some(healthy.url.clone()),
                            ..Default::default()
                        },
                    ),
                ]),
            )
            .unwrap();

            let catalog = hosts.catalog().await.unwrap();
            assert_eq!(catalog.tools.len(), 1, "{failure}");
            assert!(catalog.tools[0].description.contains("healthy/usable"));
            assert_eq!(hosts.routes.read().await.len(), 1, "{failure}");
            assert!(
                hosts
                    .routes
                    .read()
                    .await
                    .values()
                    .all(|route| route.0 == "healthy")
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| { status.alias() == "failed" && !status.available() })
            );
            assert!(
                catalog
                    .statuses
                    .iter()
                    .any(|status| { status.alias() == "healthy" && status.available() })
            );
            hosts.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn multipage_discovery_serializes_same_alias_while_other_alias_proceeds() {
        let slow = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("target")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"tools":[tool("target")],"nextCursor":"p2"}}),
            ),
            Step::Read,
            Step::Sleep(400),
            Step::Write(json!({"jsonrpc":"2.0","id":4,"result":{"tools":[tool("extra")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"slow done"}]}}),
            ),
            Step::Eof,
        ]);
        let independent = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("other")]}})),
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":3,"result":{"tools":[tool("other")]}})),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"other done"}]}}),
            ),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([
                    (
                        "independent".into(),
                        McpConfig {
                            command: Some(independent.command().into()),
                            ..Default::default()
                        },
                    ),
                    (
                        "slow".into(),
                        McpConfig {
                            command: Some(slow.command().into()),
                            ..Default::default()
                        },
                    ),
                ]),
            )
            .unwrap(),
        );
        let initial = hosts.catalog().await.unwrap();
        let slow_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.contains("slow/target"))
            .unwrap()
            .name
            .clone();
        let independent_name = initial
            .tools
            .iter()
            .find(|tool| tool.description.contains("independent/other"))
            .unwrap()
            .name
            .clone();

        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        slow.wait_for_requests(4).await;
        let slow_hosts = hosts.clone();
        let same_alias =
            tokio::spawn(async move { slow_hosts.execute(&slow_name, json!({})).await });
        let other_hosts = hosts.clone();
        let other_alias =
            tokio::spawn(async move { other_hosts.execute(&independent_name, json!({})).await });

        let McpExecution::Success(other) =
            tokio::time::timeout(std::time::Duration::from_secs(1), other_alias)
                .await
                .expect("independent alias was blocked by another alias")
                .unwrap()
                .unwrap()
        else {
            panic!("expected independent MCP success");
        };
        assert_eq!(other["content"][0]["text"], "other done");
        assert!(!same_alias.is_finished());
        assert_eq!(catalog.await.unwrap().unwrap().tools.len(), 3);
        let McpExecution::Success(slow_result) = same_alias.await.unwrap().unwrap() else {
            panic!("expected serialized MCP success");
        };
        assert_eq!(slow_result["content"][0]["text"], "slow done");
        hosts.shutdown().await.unwrap();
        let slow_conversations = slow.conversations();
        let slow_methods: Vec<_> = slow_conversations[0]
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            slow_methods,
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/list",
                "tools/list",
                "tools/call"
            ]
        );
    }

    #[tokio::test]
    async fn repeated_shutdown_waits_for_pending_client_cleanup() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.catalog().await.unwrap();

        let active = hosts.clients["stdio"].state.lock().await;
        let first_hosts = hosts.clone();
        let first = tokio::spawn(async move { first_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        let second_hosts = hosts.clone();
        let second = tokio::spawn(async move { second_hosts.shutdown().await });
        tokio::task::yield_now().await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());
        drop(active);

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert!(hosts.catalog().await.is_err());
        script.assert_completed(1);
    }

    #[tokio::test]
    async fn shutdown_interrupts_active_call_and_rejects_later_admission() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[tool("slow")]}})),
            Step::Read,
            Step::Sleep(5_000),
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let name = hosts.specs().await.unwrap()[0].name.clone();
        let call_hosts = hosts.clone();
        let call_name = name.clone();
        let call = tokio::spawn(async move { call_hosts.execute(&call_name, json!({})).await });
        script.wait_for_requests(4).await;

        tokio::time::timeout(std::time::Duration::from_secs(8), hosts.shutdown())
            .await
            .expect("active MCP shutdown exceeded the owned cleanup bound")
            .unwrap();
        let failure = call.await.unwrap().unwrap_err();
        assert_eq!(failure.kind, ToolFailureKind::McpCall);
        assert!(hosts.catalog().await.is_err());
        assert_eq!(
            hosts.execute(&name, json!({})).await.unwrap_err().kind,
            ToolFailureKind::McpRoute
        );
        assert_eq!(script.conversations()[0].len(), 4);
    }

    #[tokio::test]
    async fn shutdown_closes_admission_before_waiting_catalog_can_start() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        let held = hosts.clients["stdio"].state.lock().await;
        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        tokio::task::yield_now().await;
        let shutdown_hosts = hosts.clone();
        let shutdown = tokio::spawn(async move { shutdown_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        drop(held);

        assert!(catalog.await.unwrap().is_err());
        shutdown.await.unwrap().unwrap();
        assert!(script.conversations().is_empty());
    }

    #[tokio::test]
    async fn cancelled_pending_startup_is_cleaned_before_explicit_recovery_spawns() {
        let script = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":VERSION,"capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.admission.pause_launch.store(true, Ordering::Release);
        let launched = hosts.admission.launched.notified();
        tokio::pin!(launched);
        let catalog_hosts = hosts.clone();
        let pending = tokio::spawn(async move { catalog_hosts.catalog().await });
        launched.await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while script.conversations() != vec![Vec::<Value>::new()] {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled MCP peer did not create its empty transcript");
        pending.abort();
        match pending.await {
            Err(error) => assert!(error.is_cancelled()),
            Ok(_) => panic!("pending catalog was not cancelled"),
        }
        hosts.admission.resume_launch.notify_waiters();

        let recovered = hosts.catalog().await.unwrap();
        assert!(recovered.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let conversations = script.conversations();
        assert_eq!(conversations.len(), 2);
        let mut lengths = conversations.iter().map(Vec::len).collect::<Vec<_>>();
        lengths.sort_unstable();
        assert_eq!(lengths, [0, 3]);
        assert_eq!(conversations.iter().map(Vec::len).sum::<usize>(), 3);
    }

    #[tokio::test]
    async fn shutdown_waits_for_admitted_startup_cleanup_before_success() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = Arc::new(
            McpHosts::new(
                Path::new("."),
                &BTreeMap::from([(
                    "stdio".into(),
                    McpConfig {
                        command: Some(script.command().into()),
                        ..Default::default()
                    },
                )]),
            )
            .unwrap(),
        );
        hosts.admission.pause_launch.store(true, Ordering::Release);
        let launched = hosts.admission.launched.notified();
        tokio::pin!(launched);
        let catalog_hosts = hosts.clone();
        let catalog = tokio::spawn(async move { catalog_hosts.catalog().await });
        launched.await;
        let shutdown_hosts = hosts.clone();
        let shutdown = tokio::spawn(async move { shutdown_hosts.shutdown().await });
        while !hosts.admission.is_closed() {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        hosts.admission.resume_launch.notify_waiters();

        let catalog = catalog.await.unwrap().unwrap();
        assert!(!catalog.statuses[0].available());
        shutdown.await.unwrap().unwrap();
        assert!(hosts.catalog().await.is_err());
        assert!(script.conversations().iter().all(Vec::is_empty));
    }

    #[tokio::test]
    async fn post_spawn_setup_failure_retains_owner_until_cleanup_is_confirmed() {
        let script = StdioFixture::new([Step::Read]);
        let hosts = McpHosts::new(
            Path::new("."),
            &BTreeMap::from([(
                "stdio".into(),
                McpConfig {
                    command: Some(script.command().into()),
                    ..Default::default()
                },
            )]),
        )
        .unwrap();
        hosts.admission.fail_setup.store(true, Ordering::Release);

        let catalog = hosts.catalog().await.unwrap();
        assert!(!catalog.statuses[0].available());
        assert!(
            hosts.clients["stdio"]
                .state
                .lock()
                .await
                .transport
                .is_none()
        );
        hosts.shutdown().await.unwrap();
        assert!(script.conversations().iter().all(Vec::is_empty));
    }

    #[test]
    fn sse_parses_fragmented_crlf_multiline_and_bounds_malformed_streams() {
        let mut parser = Sse::default();
        assert!(
            parser
                .push(b": hello\r\ndata: {\r\ndata: \"id\": 1,\r\ndata: \"result\":")
                .unwrap()
                .is_empty()
        );
        let values = parser.push(b" {}\r\ndata: }\r\n\r\n").unwrap();
        assert_eq!(
            values,
            json!([{ "id":1,"result":{} }]).as_array().unwrap().clone()
        );
        assert!(Sse::default().push(b"data: invalid\n\n").is_err());
        assert!(Sse::default().push(&[255, b'\n']).is_err());
        assert!(Sse::default().push(&vec![b'a'; MAX_BYTES + 1]).is_err());
    }

    #[tokio::test]
    async fn http_sse_answers_ping_rejects_unnegotiated_requests_and_ignores_notifications() {
        let mut stream = Reply::sse(json!({"tools":[]}));
        stream.body = format!(
            "data: {{\"method\":\"notice\"}}\n\ndata: {{\"id\":900,\"method\":\"ping\"}}\n\ndata: {{\"id\":901,\"method\":\"sampling/createMessage\"}}\n\n{}",
            stream.body
        );
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            stream,
            Reply::json(json!({})),
            Reply::json(json!({})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        assert!(hosts.specs().await.unwrap().is_empty());
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(
            requests[3].body,
            json!({"jsonrpc":"2.0","id":900,"result":{}})
        );
        assert_eq!(requests[4].body["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn http_aborted_stream_and_rejected_delete_surface_failures() {
        let mut stream = Reply::sse(json!({}));
        stream.body = ": ended without response\n\n".into();
        let mut failed_delete = Reply::json(json!({}));
        failed_delete.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            stream,
            failed_delete,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.tools.is_empty());
        assert!(!catalog.statuses[0].available());
        hosts.shutdown().await.unwrap();
        let mut failed_delete = Reply::json(json!({}));
        failed_delete.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[]})),
            failed_delete,
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        hosts.specs().await.unwrap();
        assert!(
            hosts
                .shutdown()
                .await
                .unwrap_err()
                .to_string()
                .contains("DELETE failed")
        );
        hosts.shutdown().await.unwrap();
    }
}
