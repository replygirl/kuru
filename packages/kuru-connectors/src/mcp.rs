use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use futures::future::join_all;
use kuru_core::{McpConfig, ToolSpec};
use kuru_platform::fs::Directory;
#[cfg(test)]
use kuru_platform::fs::{NameRetention, Privacy};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, RwLock};

use crate::{IO_TIMEOUT, MAX_BYTES, http, rpc::Rpc, tool_output::ToolFailureKind};

const VERSION: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];

pub(crate) struct McpHosts {
    clients: BTreeMap<String, Arc<McpClient>>,
    routes: RwLock<BTreeMap<String, (String, String)>>,
    admission: Arc<Admission>,
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

pub struct McpStatus {
    alias: String,
    available: bool,
    diagnostic: Option<String>,
}

impl McpStatus {
    pub fn alias(&self) -> &str {
        &self.alias
    }

    pub fn available(&self) -> bool {
        self.available
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

pub(crate) struct McpCatalog {
    pub(crate) tools: Vec<ToolSpec>,
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
        let admission = Arc::new(Admission::new());
        for (name, config) in configs {
            ensure!(
                config.command.is_some() != config.url.is_some(),
                "MCP {name} needs exactly one command or URL"
            );
            if let Some(url) = &config.url {
                http::endpoint(url)?;
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
            routes: RwLock::new(BTreeMap::new()),
            admission,
        })
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
            let tools = match client.list(&mut state).await {
                Ok(tools) => tools,
                Err(_) => {
                    if !state.close_attempted {
                        let _ = close_transport(&mut state).await;
                    }
                    return Ok::<_, anyhow::Error>((
                        Vec::new(),
                        McpStatus {
                            alias: alias.clone(),
                            available: false,
                            diagnostic: state.diagnostic.clone(),
                        },
                    ));
                }
            };
            let candidate = (|| {
                let mut routes = BTreeMap::new();
                let mut toolspecs = Vec::new();
                for tool in tools {
                    let original = tool["name"].as_str().context("MCP tool lacks name")?;
                    let key = format!(
                        "mcp_{}",
                        uuid::Uuid::new_v5(
                            &uuid::Uuid::NAMESPACE_URL,
                            format!("{alias}\0{original}").as_bytes()
                        )
                        .simple()
                    );
                    ensure!(
                        routes
                            .insert(key.clone(), (alias.clone(), original.into()))
                            .is_none(),
                        "duplicate MCP tool name for {alias}"
                    );
                    ensure!(
                        tool["inputSchema"].is_object(),
                        "MCP tool lacks inputSchema object"
                    );
                    toolspecs.push(ToolSpec {
                        name: key,
                        description: format!(
                            "MCP {alias}/{original}: {}",
                            tool["description"]
                                .as_str()
                                .unwrap_or("Configured external tool")
                        ),
                        parameters: tool["inputSchema"].clone(),
                    });
                }
                Ok::<_, anyhow::Error>((routes, toolspecs))
            })();
            match candidate {
                Ok((candidate_routes, candidate_specs)) => {
                    let mut routes = self.routes.write().await;
                    if candidate_routes
                        .keys()
                        .any(|name| routes.get(name).is_some_and(|(owner, _)| owner != alias))
                    {
                        drop(routes);
                        let _ = close_transport(&mut state).await;
                        return Ok::<_, anyhow::Error>((
                            Vec::new(),
                            McpStatus {
                                alias: alias.clone(),
                                available: false,
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
                    Ok::<_, anyhow::Error>((
                        candidate_specs,
                        McpStatus {
                            alias: alias.clone(),
                            available: true,
                            diagnostic: None,
                        },
                    ))
                }
                Err(_) => {
                    let _ = close_transport(&mut state).await;
                    Ok::<_, anyhow::Error>((
                        Vec::new(),
                        McpStatus {
                            alias: alias.clone(),
                            available: false,
                            diagnostic: state.diagnostic.clone(),
                        },
                    ))
                }
            }
        });
        let mut specs = Vec::new();
        let mut statuses = Vec::with_capacity(self.clients.len());
        for result in join_all(discoveries).await {
            let (candidate_specs, status) = result?;
            specs.extend(candidate_specs);
            statuses.push(status);
        }
        Ok(McpCatalog {
            tools: specs,
            statuses,
        })
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

    async fn initialize(&self, state: &mut ClientState) -> Result<()> {
        state.transport = Some(if let Some(url) = &self.config.url {
            Transport::Http(HttpRpc {
                client: http::client()?,
                url: url.clone(),
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
            self.initialize(state).await?;
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

    async fn list(&self, state: &mut ClientState) -> Result<Vec<Value>> {
        if !self.available.load(Ordering::Acquire) {
            close_transport(state).await?;
        }
        state.diagnostic = None;
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
                    .list(&mut state)
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
        let peer = HttpFixture::new(vec![
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
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = hosts.specs().await.unwrap()[0].name.clone();
        assert!(hosts.execute(&name, json!({})).await.is_err());
        assert_eq!(peer.requests.lock().await.len(), 5);
        let catalog = hosts.catalog().await.unwrap();
        assert!(catalog.statuses[0].available());
        assert_eq!(catalog.tools[0].name, name);
        assert_eq!(peer.requests.lock().await.len(), 8);
        let McpExecution::Success(result) = hosts.execute(&name, json!({})).await.unwrap() else {
            panic!("expected recovered call success");
        };
        assert_eq!(result["content"][0]["text"], "recovered");
        hosts.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 10);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .count(),
            2
        );
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
            let failed = StdioFixture::new([Step::Read, Step::Raw("not JSON")]);
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
            failed.assert_completed(1);
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
        assert_eq!(conversations.len(), 1);
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
