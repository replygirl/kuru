use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{McpConfig, ToolSpec};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

use crate::{IO_TIMEOUT, MAX_BYTES, http, rpc::Rpc};

const VERSION: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];

pub(crate) struct McpHosts {
    clients: BTreeMap<String, Arc<McpClient>>,
    routes: RwLock<BTreeMap<String, (String, String)>>,
}

impl McpHosts {
    pub fn new(root: &Path, configs: &BTreeMap<String, McpConfig>) -> Result<Self> {
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
                    root: root.into(),
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

    pub async fn execute(&self, name: &str, arguments: Value) -> Result<String> {
        let route = self.routes.read().await.get(name).cloned();
        let (alias, original) = route.with_context(|| {
            format!("unknown tool: {name}; discover configured MCP tools first")
        })?;
        let client = self
            .clients
            .get(&alias)
            .context("MCP route no longer exists")?;
        let result = client.call(&original, arguments).await?;
        ensure!(
            result["isError"] != true,
            "MCP {alias}/{original} returned a tool error: {}",
            result.get("content").unwrap_or(&Value::Null)
        );
        Ok(result.to_string())
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
            Transport::Stdio(Box::new(Rpc::spawn(
                self.config
                    .command
                    .as_deref()
                    .context("missing MCP command")?,
                &self.config.args,
                &self.config.env,
                &self.root,
            )?))
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
            Self::Stdio(rpc) => {
                rpc.close().await;
                Ok(())
            }
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
    use crate::test_support::{HttpFixture, Reply, StdioFixture, Step};

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
        let value: Value = serde_json::from_str(
            &hosts
                .execute(&specs[0].name, json!({"path":"a"}))
                .await
                .unwrap(),
        )
        .unwrap();
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
        let one: Value = serde_json::from_str(&one.unwrap()).unwrap();
        let two: Value = serde_json::from_str(&two.unwrap()).unwrap();
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
            let client = McpClient {
                config: http_config(&peer.url).remove("test").unwrap(),
                root: PathBuf::from("."),
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
    async fn tool_error_and_protocol_failure_are_distinct_and_not_automatically_retried() {
        let peer = HttpFixture::new(vec![
            initialized(),
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[tool("a")]})),
            Reply::rpc(json!({"isError":true,"content":[{"type":"text","text":"denied"}]})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":"server failed"}})),
            Reply::json(json!({})),
        ])
        .await;
        let hosts = McpHosts::new(Path::new("."), &http_config(&peer.url)).unwrap();
        let name = &hosts.specs().await.unwrap()[0].name;
        assert!(
            hosts
                .execute(name, json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("tool error")
        );
        assert!(
            hosts
                .execute(name, json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("server failed")
        );
        assert_eq!(peer.requests.lock().await.len(), 6);
        hosts.shutdown().await.unwrap();
        assert!(hosts.execute("unknown", json!({})).await.is_err());
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
