use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Recorded {
    pub body: Value,
    pub headers: HeaderMap,
    pub method: Method,
}
pub struct Reply {
    pub body: String,
    pub status: StatusCode,
    pub content_type: &'static str,
    pub session: bool,
}
impl Reply {
    pub fn json(value: Value) -> Self {
        Self {
            body: value.to_string(),
            status: StatusCode::OK,
            content_type: "application/json",
            session: false,
        }
    }
    pub fn rpc(value: Value) -> Self {
        Self::json(json!({"jsonrpc":"2.0","id":"$ID","result":value}))
    }
    pub fn sse(value: Value) -> Self {
        Self {
            body: format!(
                ": comment\r\ndata: {}\r\n\r\n",
                json!({"jsonrpc":"2.0","id":"$ID","result":value})
            ),
            status: StatusCode::OK,
            content_type: "text/event-stream",
            session: false,
        }
    }
}

#[derive(Clone)]
struct Queue {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}
pub struct HttpFixture {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    task: tokio::task::JoinHandle<()>,
}
impl HttpFixture {
    pub async fn new(replies: Vec<Reply>) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Queue {
            replies: Arc::new(Mutex::new(replies.into())),
            requests: requests.clone(),
        };
        let router = Router::new().fallback(any(handle)).with_state(state);
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(socket, router).await.unwrap();
        });
        Self {
            url,
            requests,
            task,
        }
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn handle(
    State(state): State<Queue>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    state.requests.lock().await.push(Recorded {
        body: body.clone(),
        headers,
        method,
    });
    let reply = state
        .replies
        .lock()
        .await
        .pop_front()
        .expect("unexpected fixture request");
    let text = reply.body.replace("\"$ID\"", &body["id"].to_string());
    let mut response = (reply.status, [("Content-Type", reply.content_type)], text).into_response();
    if reply.session {
        response
            .headers_mut()
            .insert("Mcp-Session-Id", "fixture-session".parse().unwrap());
    }
    response
}

pub struct Script {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
}
impl Script {
    pub fn new(source: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture");
        std::fs::write(&path, format!("#!/usr/bin/env python3\n{source}")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            _directory: directory,
            path,
        }
    }
    pub fn command(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

pub fn request() -> kuru_core::CompletionRequest {
    kuru_core::CompletionRequest {
        actor: "project/ifs/part-a".into(),
        instructions: "Actor-specific context".into(),
        messages: vec![kuru_core::Message {
            role: "user".into(),
            content: "hello".into(),
        }],
        model: "future-model".into(),
        effort: Some("future-effort".into()),
        tools: vec![kuru_core::ToolSpec {
            name: "file_read".into(),
            description: "Read".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        }],
    }
}
