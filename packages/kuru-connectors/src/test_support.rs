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
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
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
    // The fixture directory must disappear before its containing artifact.
    _binary: Arc<CompiledPeer>,
}
impl StdioFixture {
    pub fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        static BINARY: OnceLock<FixtureCache> = OnceLock::new();
        Self::with_cache(steps, BINARY.get_or_init(FixtureCache::default))
    }

    fn with_cache(steps: impl IntoIterator<Item = Step>, cache: &FixtureCache) -> Self {
        let binary = fixture_binary(cache);
        // Keeping both paths on the same filesystem makes linking reliable even
        // when the test process's workspace and temporary directory differ.
        let directory = tempfile::tempdir_in(binary.directory.path()).unwrap();
        let path = directory.path().join("fixture");
        std::fs::hard_link(&binary.path, &path).unwrap();
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
        Self {
            directory,
            path,
            _binary: binary,
        }
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

    fn diagnostics(&self) -> String {
        use std::io::Read;
        let mut text = format!("native fixture {}", self.path.display());
        let Ok(entries) = std::fs::read_dir(self.directory.path()) else {
            return text;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    ["started", "error", "requests", "done", "plan"]
                        .iter()
                        .any(|value| extension == *value)
                })
            })
            .collect();
        paths.sort();
        for path in paths.into_iter().take(8) {
            let mut bytes = Vec::new();
            if let Ok(file) = std::fs::File::open(&path)
                && file.take(2048).read_to_end(&mut bytes).is_ok()
            {
                text.push_str(&format!(
                    "\n{}: {}",
                    path.display(),
                    String::from_utf8_lossy(&bytes)
                ));
            }
        }
        text
    }
}

impl Drop for StdioFixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("{}", self.diagnostics());
        }
    }
}

struct CompiledPeer {
    directory: tempfile::TempDir,
    path: PathBuf,
}

type FixtureCache = StdMutex<Weak<CompiledPeer>>;

fn fixture_binary(cache: &FixtureCache) -> Arc<CompiledPeer> {
    let mut cached = cache.lock().unwrap();
    if let Some(binary) = cached.upgrade() {
        return binary;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("stdio_peer.rs");
    let path = directory.path().join("stdio_peer");
    std::fs::write(&source, include_str!("../tests/fixtures/stdio_peer.rs")).unwrap();
    // Only the compiler opens executable bytes for writing. Wait for it to exit
    // before publishing paths: the multithreaded test process never holds a
    // writable executable descriptor that another fork could inherit (ETXTBSY).
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
            .arg(&path)
            .output()
            .expect("compile the native connector test peer with the Rust toolchain");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    }
    let binary = Arc::new(CompiledPeer { directory, path });
    // A static strong reference would prevent TempDir cleanup at process exit.
    *cached = Arc::downgrade(&binary);
    binary
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

#[cfg(unix)]
mod tests {
    use super::{FixtureCache, StdioFixture, Step};
    use serde_json::{Value, json};
    use std::{os::unix::fs::MetadataExt, path::Path, process::Stdio, time::Duration};
    use tokio::{
        io::AsyncReadExt, io::AsyncWriteExt, process::Command, sync::Barrier, time::timeout,
    };

    async fn exchange(fixture: &StdioFixture, request: Value, started: &Barrier) -> Value {
        exchange_program(fixture, &fixture.path, request, started).await
    }

    async fn exchange_program(
        fixture: &StdioFixture,
        program: &Path,
        request: Value,
        started: &Barrier,
    ) -> Value {
        let mut child = Command::new(program)
            .arg0(&fixture.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the native fixture through its own executable path");
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let mut errors = child.stderr.take().unwrap();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let result = timeout(Duration::from_secs(10), async {
            // Both subprocesses must be alive before either receives its input.
            started.wait().await;
            tokio::join!(
                async {
                    let result = input.write_all(format!("{request}\n").as_bytes()).await;
                    drop(input);
                    result
                },
                child.wait(),
                output.read_to_string(&mut stdout),
                errors.read_to_string(&mut stderr),
            )
        })
        .await;
        let status = match result {
            Ok((Ok(()), Ok(status), Ok(_), Ok(_))) => status,
            failure => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                panic!("native fixture failed: {failure:?}; stdout={stdout:?}; stderr={stderr:?}");
            }
        };
        assert!(
            status.success(),
            "native fixture failed: {status}; {stderr}"
        );
        serde_json::from_str(&stdout).expect("the fixture must return its exact JSON plan")
    }

    #[tokio::test]
    async fn fixture_uses_absolute_invocation_path_for_its_wire_plan() {
        let fixture = StdioFixture::with_cache(
            [
                Step::Read,
                Step::Write(json!({"identity": "invocation"})),
                Step::Eof,
            ],
            &FixtureCache::default(),
        );
        assert!(fixture.path.is_absolute());
        assert!(!fixture._binary.path.with_extension("plan").exists());
        let request = json!({"id": "invocation", "method": "identity"});
        let started = Barrier::new(1);
        let reply =
            exchange_program(&fixture, &fixture._binary.path, request.clone(), &started).await;
        assert_eq!(reply, json!({"identity": "invocation"}));
        fixture.assert_completed(1);
        assert_eq!(fixture.conversations(), vec![vec![request]]);
    }

    #[tokio::test]
    async fn fixture_records_unexpected_failure_when_stderr_is_discarded() {
        let fixture = StdioFixture::new([Step::Read]);
        let mut child = Command::new(&fixture.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let status = match timeout(Duration::from_secs(10), child.wait()).await {
            Ok(result) => result.unwrap(),
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                panic!(
                    "fixture did not report closed input: {error}; {}",
                    fixture.diagnostics()
                );
            }
        };
        assert_eq!(status.code(), Some(1));
        let diagnostics = fixture.diagnostics();
        assert!(diagnostics.contains(".started:"), "{diagnostics}");
        assert!(diagnostics.contains("invocation="), "{diagnostics}");
        assert!(
            diagnostics.contains(".error: unexpected input during read"),
            "{diagnostics}"
        );
        assert_eq!(fixture.conversations(), vec![Vec::<Value>::new()]);
    }

    #[tokio::test]
    async fn stdio_fixtures_share_executable_and_isolate_concurrent_plans() {
        let first = StdioFixture::new([
            Step::Read,
            Step::Write(json!({"result": "first"})),
            Step::Eof,
        ]);
        let second = StdioFixture::new([
            Step::Read,
            Step::Write(json!({"result": "second"})),
            Step::Eof,
        ]);
        let first_metadata = std::fs::metadata(&first.path).unwrap();
        let second_metadata = std::fs::metadata(&second.path).unwrap();
        assert_eq!(
            (first_metadata.dev(), first_metadata.ino()),
            (second_metadata.dev(), second_metadata.ino()),
            "fixtures must share an immutable executable instead of opening new executables for writing",
        );
        assert_eq!(first_metadata.mode() & 0o222, 0);
        let first_request = json!({"id": 1, "method": "first"});
        let second_request = json!({"id": 2, "method": "second"});
        let started = Barrier::new(2);
        let (first_reply, second_reply) = tokio::join!(
            exchange(&first, first_request.clone(), &started),
            exchange(&second, second_request.clone(), &started),
        );
        assert_eq!(first_reply, json!({"result": "first"}));
        assert_eq!(second_reply, json!({"result": "second"}));
        first.assert_completed(1);
        second.assert_completed(1);
        assert_eq!(first.conversations(), vec![vec![first_request]]);
        assert_eq!(second.conversations(), vec![vec![second_request]]);
    }

    #[tokio::test]
    async fn final_fixture_owner_cleans_artifact_and_cache_can_rebuild() {
        // Global-cache owners in unrelated tests must not affect this proof.
        let cache = FixtureCache::default();
        let first = StdioFixture::with_cache([], &cache);
        let second = StdioFixture::with_cache(
            [Step::Read, Step::Write(json!("survivor")), Step::Eof],
            &cache,
        );
        let artifact_directory = first._binary.directory.path().to_owned();
        let first_directory = first.directory.path().to_owned();
        let second_directory = second.directory.path().to_owned();
        assert_eq!(artifact_directory, second._binary.directory.path());

        drop(first);
        assert!(!first_directory.exists());
        assert!(artifact_directory.exists());
        let started = Barrier::new(1);
        assert_eq!(
            exchange(&second, json!({"id": "survivor"}), &started).await,
            json!("survivor"),
        );
        second.assert_completed(1);
        drop(second);
        assert!(!second_directory.exists());
        assert!(!artifact_directory.exists());
        assert!(cache.lock().unwrap().upgrade().is_none());

        let replacement = StdioFixture::with_cache(
            [Step::Read, Step::Write(json!("rebuilt")), Step::Eof],
            &cache,
        );
        let replacement_directory = replacement._binary.directory.path().to_owned();
        assert_eq!(
            exchange(&replacement, json!({"id": "rebuilt"}), &started).await,
            json!("rebuilt"),
        );
        replacement.assert_completed(1);
        drop(replacement);
        assert!(!replacement_directory.exists());
        assert!(cache.lock().unwrap().upgrade().is_none());
    }
}
