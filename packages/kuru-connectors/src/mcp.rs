use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[cfg(test)]
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{McpConfig, ToolSpec};
use kuru_platform::fs::Directory;
#[cfg(test)]
use kuru_platform::fs::{NameRetention, Privacy};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

use crate::{IO_TIMEOUT, MAX_BYTES, http, rpc::Rpc, tool_output::ToolFailureKind};

const VERSION: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];

pub(crate) struct McpHosts {
    clients: BTreeMap<String, Arc<McpClient>>,
    routes: RwLock<BTreeMap<String, (String, String)>>,
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

    fn call(error: anyhow::Error) -> Self {
        Self {
            kind: ToolFailureKind::McpCall,
            error,
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
                    transport: Mutex::new(None),
                }),
            );
        }
        Ok(Self {
            clients,
            routes: RwLock::new(BTreeMap::new()),
        })
    }

    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        let mut routes = BTreeMap::new();
        let mut specs = Vec::new();
        for (alias, client) in &self.clients {
            for tool in client
                .list()
                .await
                .with_context(|| format!("MCP {alias} discovery failed"))?
            {
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
                specs.push(ToolSpec {
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
        }
        *self.routes.write().await = routes;
        Ok(specs)
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
        let result = client
            .call(&original, arguments)
            .await
            .map_err(McpCallFailure::call)?;
        if result["isError"] == true {
            Ok(McpExecution::ApplicationError(result["content"].clone()))
        } else {
            Ok(McpExecution::Success(result))
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        let mut first_error = None;
        for client in self.clients.values() {
            if let Err(error) = client.close().await {
                first_error.get_or_insert(error);
            }
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
    transport: Mutex<Option<Transport>>,
}

impl McpClient {
    async fn initialize(&self) -> Result<Transport> {
        let mut transport = if let Some(url) = &self.config.url {
            Transport::Http(HttpRpc {
                client: http::client()?,
                url: url.clone(),
                session: None,
                version: None,
                next_id: 0,
            })
        } else {
            self.root_guard.revalidate()?;
            Transport::Stdio(Box::new(
                Rpc::spawn(
                    self.config
                        .command
                        .as_deref()
                        .context("missing MCP command")?,
                    &self.config.args,
                    &self.config.env,
                    &self.root,
                )
                .await?,
            ))
        };
        let initialization = async {
            let result = transport.request("initialize", json!({"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")}})).await?;
            let version = result["protocolVersion"].as_str().context("MCP initialize lacks protocolVersion")?;
            ensure!(SUPPORTED.contains(&version), "unsupported MCP protocol version: {version}");
            ensure!(result["capabilities"]["tools"].is_object(), "MCP server did not advertise tools capability");
            if let Transport::Http(http) = &mut transport { http.version = Some(version.into()); }
            transport.notify("notifications/initialized", json!({})).await
        }.await;
        if let Err(error) = initialization {
            let _ = transport.close().await;
            return Err(error);
        }
        Ok(transport)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let mut state = self.transport.lock().await;
        if state.is_none() {
            *state = Some(self.initialize().await?);
        }
        let result = state
            .as_mut()
            .context("MCP initialization failed")?
            .request(method, params)
            .await;
        if result.is_err() {
            // Never automatically retry a tool mutation. A subsequent explicit
            // call may establish a fresh session after a failed transport.
            if let Some(mut transport) = state.take() {
                let _ = transport.close().await;
            }
        }
        result
    }

    async fn list(&self) -> Result<Vec<Value>> {
        let mut cursor = Value::Null;
        let mut seen = BTreeSet::new();
        let mut tools = Vec::new();
        loop {
            let params = if cursor.is_null() {
                json!({})
            } else {
                json!({"cursor":cursor})
            };
            let result = self.request("tools/list", params).await?;
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

    async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))
            .await
    }

    async fn close(&self) -> Result<()> {
        if let Some(mut transport) = self.transport.lock().await.take() {
            transport.close().await?;
        }
        Ok(())
    }
}

enum Transport {
    Stdio(Box<Rpc>),
    Http(HttpRpc),
}

impl Transport {
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
            Self::Http(rpc) => {
                http::body(
                    rpc.post()
                        .json(&json!({"jsonrpc":"2.0","method":method,"params":params}))
                        .send()
                        .await?,
                )
                .await?;
                Ok(())
            }
        }
    }
    async fn close(&mut self) -> Result<()> {
        match self {
            Self::Stdio(rpc) => rpc.close().await,
            Self::Http(rpc) => {
                if let Some(session) = rpc.session.take() {
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

        let error = hosts.specs().await.unwrap_err();
        assert!(format!("{error:#}").contains("identity changed"));
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
                transport: Mutex::new(None),
            };
            assert!(
                client
                    .list()
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
        assert!(
            hosts
                .specs()
                .await
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
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
        assert!(format!("{:#}", hosts.specs().await.unwrap_err()).contains("pagination"));
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
        assert!(failure.error.to_string().contains("server failed"));
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
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        assert!(format!("{:#}", hosts.specs().await.unwrap_err()).contains("stream ended"));
        hosts.shutdown().await.unwrap();
        let mut failed_delete = Reply::json(json!({}));
        failed_delete.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[]})),
            failed_delete,
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
    }
}
