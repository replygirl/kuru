use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
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

pub enum Step {
    Read,
    Write(Value),
    Raw(&'static str),
    Repeat(usize),
    Sleep(u64),
    Auth,
    Exit(i32),
    Eof,
}

/// A real compiled peer with an independent transcript for each subprocess.
/// Request assertions belong in the tests, not a second ad-hoc JSON parser.
pub struct StdioFixture {
    directory: tempfile::TempDir,
    pub path: PathBuf,
}
impl StdioFixture {
    pub fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture");
        std::fs::write(&path, fixture_binary()).unwrap();
        let plan: String = steps
            .into_iter()
            .map(|step| match step {
                Step::Read => "read\n".into(),
                Step::Write(value) => format!("write {value}\n"),
                Step::Raw(line) => format!("write {line}\n"),
                Step::Repeat(count) => format!("repeat {count}\n"),
                Step::Sleep(milliseconds) => format!("sleep {milliseconds}\n"),
                Step::Auth => "auth\n".into(),
                Step::Exit(code) => format!("exit {code}\n"),
                Step::Eof => "eof\n".into(),
            })
            .collect();
        std::fs::write(path.with_extension("plan"), plan).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self { directory, path }
    }
    pub fn command(&self) -> &str {
        self.path.to_str().unwrap()
    }

    pub fn conversations(&self) -> Vec<Vec<Value>> {
        let mut files: Vec<_> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|value| value == "requests"))
            .collect();
        files.sort();
        files
            .iter()
            .map(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            })
            .collect()
    }

    pub fn assert_completed(&self, processes: usize) {
        let finished = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|value| value == "done"))
            .count();
        assert_eq!(
            finished, processes,
            "native peer did not finish its wire plan"
        );
        assert_eq!(self.conversations().len(), processes);
    }
}

fn fixture_binary() -> &'static [u8] {
    static BINARY: OnceLock<Vec<u8>> = OnceLock::new();
    BINARY.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("stdio_peer.rs");
        let binary = directory.path().join("stdio_peer");
        std::fs::write(&source, include_str!("../tests/fixtures/stdio_peer.rs")).unwrap();
        let result =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .args([
                    "--edition=2024",
                    "--forbid",
                    "unsafe_code",
                    "-C",
                    "opt-level=1",
                ])
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .expect("compile the native connector test peer with the Rust toolchain");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        std::fs::read(binary).unwrap()
    })
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
