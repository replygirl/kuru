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
    io,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Mutex,
};

/// Retain a bounded diagnostic prefix while continuing to drain an owned test
/// child pipe. A fixture failure must not let a noisy child grow test memory or
/// keep its parent waiting on a full pipe.
pub async fn drain_bounded(
    reader: &mut (impl AsyncRead + Unpin),
    retained: &mut Vec<u8>,
) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(truncated);
        }
        let keep = count.min(crate::MAX_BYTES.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep != count;
    }
}

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
    Stderr(&'static str),
    StderrRepeat(usize),
    StderrInvalid,
    HoldStderr(u64),
    Sleep(u64),
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
        // The image must keep a stable identity while sibling aliases retire:
        // macOS can reject shared hardlinks if policy inspection races cleanup.
        // argv[0] still selects this fixture's independent plan and transcript.
        let directory = tempfile::tempdir_in(binary.directory.path()).unwrap();
        let path = directory.path().join(if cfg!(windows) {
            "fixture.exe"
        } else {
            "fixture"
        });
        #[cfg(unix)]
        std::os::unix::fs::symlink(&binary.path, &path).unwrap();
        #[cfg(not(unix))]
        std::fs::hard_link(&binary.path, &path).unwrap();
        let plan: String = steps
            .into_iter()
            .map(|step| match step {
                Step::Read => "read\n".into(),
                Step::Write(value) => format!("write {value}\n"),
                Step::Raw(line) => format!("write {line}\n"),
                Step::Repeat(count) => format!("repeat {count}\n"),
                Step::Stderr(line) => format!("stderr {line}\n"),
                Step::StderrRepeat(count) => format!("stderr-repeat {count}\n"),
                Step::StderrInvalid => "stderr-invalid\n".into(),
                Step::HoldStderr(milliseconds) => format!("hold-stderr {milliseconds}\n"),
                Step::Sleep(milliseconds) => format!("sleep {milliseconds}\n"),
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

    pub async fn wait_for_requests(&self, count: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if self.conversations().iter().map(Vec::len).sum::<usize>() >= count {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("request observation timed out: {}", self.diagnostics()));
    }

    #[cfg(unix)]
    pub fn environment_observations(&self) -> Vec<String> {
        let mut paths: Vec<_> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|value| value == "environment"))
            .collect();
        paths.sort();
        paths
            .into_iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
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
    #[cfg(windows)]
    let path = snapshot_peer(&cargo_peer(), directory.path())
        .expect("snapshot the Cargo-built native connector test peer");
    #[cfg(unix)]
    let path = {
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
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        }
        path
    };
    let binary = Arc::new(CompiledPeer { directory, path });
    // A static strong reference would prevent TempDir cleanup at process exit.
    *cached = Arc::downgrade(&binary);
    binary
}

#[cfg(windows)]
fn cargo_peer() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("kuru-connectors-stdio-fixture.exe")
}

#[cfg(windows)]
fn snapshot_peer(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    use anyhow::{Context, ensure};
    use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
    use std::{
        ffi::OsStr,
        fs::File,
        io::{Read, Seek, Write},
        os::windows::fs::MetadataExt,
    };

    const LIMIT: u64 = 64 * 1024 * 1024;
    fn read(file: &mut File) -> anyhow::Result<Vec<u8>> {
        file.rewind()?;
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "native fixture exceeds snapshot limit"
        );
        Ok(bytes)
    }
    fn open(path: &std::path::Path) -> anyhow::Result<File> {
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && metadata.file_attributes() & 0x400 == 0,
            "native fixture source must be a regular non-reparse file"
        );
        let file = File::open(path)?;
        regular_file_info(&file)?;
        Ok(file)
    }

    // Cargo's image and the OS temporary directory can be on different volumes.
    // Snapshot once into the owning directory before making same-volume aliases.
    // Cargo may legitimately hard-link its source; never modify that artifact.
    let parent = Directory::open(
        source.parent().context("fixture source has no parent")?,
        Privacy::Inherited,
        NameRetention::Pinned,
    )?;
    let mut input = open(source)?;
    let before = regular_file_info(&input)?;
    ensure!(
        before.len != 0 && before.len <= LIMIT,
        "invalid native fixture size"
    );
    let bytes = read(&mut input)?;
    let named = open(source)?;
    let after = regular_file_info(&input)?;
    let current = regular_file_info(&named)?;
    ensure!(
        before.identity == after.identity
            && before.identity == current.identity
            && before.len == after.len
            && before.len == current.len
            && before.len == bytes.len() as u64
            && read(&mut input)? == bytes,
        "native fixture changed while snapshotting"
    );
    ensure!(
        Directory::open(parent.path(), Privacy::Inherited, NameRetention::Pinned)?.identity()
            == parent.identity(),
        "native fixture source parent changed"
    );
    let output = Directory::open(destination, Privacy::Inherited, NameRetention::Pinned)?;
    let name = OsStr::new("stdio_peer.exe");
    let mut candidate = output.create_new(name)?;
    candidate.write_all(&bytes)?;
    candidate.sync_all()?;
    ensure!(
        read(&mut candidate)? == bytes,
        "native fixture snapshot differs from source"
    );
    output.verify(name, &candidate)?;
    Ok(output.path().join(name))
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
        for program in [&fixture._binary.path, &fixture.path] {
            let reply = exchange_program(&fixture, program, request.clone(), &started).await;
            assert_eq!(reply, json!({"identity": "invocation"}));
        }
        fixture.assert_completed(2);
        assert_eq!(
            fixture.conversations(),
            vec![vec![request.clone()], vec![request]],
        );

        let canonical_executable = fixture._binary.path.canonicalize().unwrap();
        let identities: Vec<_> = std::fs::read_dir(fixture.directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "started")
            })
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect();
        assert_eq!(identities.len(), 2);
        for identity in identities {
            assert!(
                identity.contains(&format!("invocation={:?}", fixture.path)),
                "wire plans must retain their independent invocation path: {identity}",
            );
            // macOS may report the invocation symlink, whereas Linux resolves
            // it. In either case the reported image must resolve to the shared
            // artifact, not a disposable hardlink with a different pathname.
            let reported_image = [&fixture.path, &fixture._binary.path, &canonical_executable]
                .into_iter()
                .find(|path| {
                    identity
                        .lines()
                        .any(|line| line == format!("current_exe=Ok({path:?})"))
                })
                .unwrap_or_else(|| panic!("unexpected executable identity: {identity}"));
            assert_eq!(
                reported_image.canonicalize().unwrap(),
                canonical_executable,
                "the running image must resolve to the shared artifact after sibling aliases retire: {identity}",
            );
        }
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

#[cfg(windows)]
mod windows_tests {
    use super::{CompiledPeer, FixtureCache, StdioFixture, Step, cargo_peer, snapshot_peer};
    use crate::{mcp::Admission, rpc::Rpc};
    use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
    use serde_json::json;
    use std::{collections::BTreeMap, fs::File, sync::Arc, time::Duration};

    #[tokio::test]
    async fn native_fixture_snapshot_survives_source_replacement_and_cleans_its_aliases() {
        let sources = tempfile::tempdir().unwrap();
        let source = sources.path().join("cargo-peer.exe");
        std::fs::copy(cargo_peer(), &source).unwrap();
        let compiled_link = sources.path().join("cargo-artifact.exe");
        std::fs::hard_link(&source, &compiled_link).unwrap();
        let original = File::open(&source).unwrap();
        assert_eq!(regular_file_info(&original).unwrap().links, 2);
        let directory = tempfile::tempdir().unwrap();
        let path = snapshot_peer(&source, directory.path()).unwrap();
        let snapshot = File::open(&path).unwrap();
        assert_ne!(
            regular_file_info(&original).unwrap().identity,
            regular_file_info(&snapshot).unwrap().identity
        );
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&path).unwrap()
        );
        drop((original, snapshot));
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"replacement must never execute").unwrap();
        assert_eq!(
            std::fs::read(&compiled_link).unwrap(),
            std::fs::read(&path).unwrap()
        );
        let owned_directory = directory.path().to_owned();
        let binary = Arc::new(CompiledPeer { directory, path });
        let cache = FixtureCache::new(Arc::downgrade(&binary));
        let fixture = StdioFixture::with_cache(
            [Step::Read, Step::Write(json!({"snapshot":true})), Step::Eof],
            &cache,
        );
        let source_image = File::open(&binary.path).unwrap();
        let alias = File::open(&fixture.path).unwrap();
        assert_eq!(
            regular_file_info(&source_image).unwrap().identity,
            regular_file_info(&alias).unwrap().identity
        );
        drop((source_image, alias));
        let mut rpc = Rpc::spawn(
            fixture.command(),
            &[],
            &BTreeMap::new(),
            fixture.directory.path(),
            Arc::new(
                Directory::open(
                    fixture.directory.path(),
                    Privacy::Inherited,
                    NameRetention::Pinned,
                )
                .unwrap(),
            ),
            Arc::new(Admission::new()),
        )
        .unwrap();
        rpc.ready().await.unwrap();
        let exchange = tokio::time::timeout(Duration::from_secs(10), async {
            rpc.send(json!({"request":"retained snapshot"}))
                .await
                .unwrap();
            rpc.read().await.unwrap()
        })
        .await;
        rpc.close().await.unwrap();
        assert_eq!(exchange.unwrap(), json!({"snapshot":true}));
        fixture.assert_completed(1);
        assert_eq!(
            fixture.conversations(),
            vec![vec![json!({"request":"retained snapshot"})]]
        );
        drop(rpc);
        drop(fixture);
        drop(binary);
        assert!(!owned_directory.exists());
        assert!(cache.lock().unwrap().upgrade().is_none());
    }
}
