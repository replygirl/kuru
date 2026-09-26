//! Real native mise GitHub-backend acceptance with simulated release metadata.
//! Compiled by the app integration wrapper, so the candidate is the actual Kuru
//! executable and delivery acquires no reverse runtime dependency.

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Method, Request, Response, StatusCode},
    routing::any,
};
use kuru_delivery::{
    archive,
    command::{self, Command},
    published_windows, shell_support, targets,
};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use kuru_platform::windows::process::configured_command;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::{Arc, Mutex},
    time::Duration,
};

#[path = "../../src/mise_isolation.rs"]
mod mise_isolation;

const DEADLINE: Duration = Duration::from_secs(180);
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TARGET: &str = "x86_64-pc-windows-msvc";
fn checked_file_within(path: &Path, root: &Path) -> Result<bool> {
    let ancestor = Directory::open(root, Privacy::Inherited, NameRetention::Movable)?;
    let parent = Directory::open(
        path.parent().context("fixture path has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let name = path.file_name().context("fixture path has no filename")?;
    let file = parent.read(name)?;
    parent.verify(name, &file)?;
    Ok(parent.is_within(&ancestor)?)
}

#[derive(Clone, Copy)]
struct Scenario {
    api_digest: bool,
    corrupt: bool,
    bad_checksum: bool,
    api_fallback: bool,
    missing: bool,
}
impl Scenario {
    const VALID: Self = Self {
        api_digest: true,
        corrupt: false,
        bad_checksum: false,
        api_fallback: false,
        missing: false,
    };
}

struct Traffic {
    method: Method,
    path: String,
    bytes: usize,
}
struct ServerData {
    scenario: Scenario,
    archive: Vec<u8>,
    digest: String,
    requests: Vec<Traffic>,
    rejected: Vec<String>,
}
struct Server {
    base: String,
    state: Arc<Mutex<ServerData>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn asset_name() -> String {
    archive::archive_name(VERSION, TARGET).unwrap()
}
fn release(data: &ServerData) -> Value {
    let mut assets:Vec<_>=targets::CATALOG.iter().enumerate().filter(|(_,target)| !data.scenario.missing || target.triple!=TARGET).map(|(index,target)| {
        let name=archive::archive_name(VERSION,target.triple).unwrap();
        json!({"name":name,"browser_download_url":format!("https://github.com/replygirl/kuru/releases/download/v{VERSION}/{name}"),"url":format!("https://api.github.com/repos/replygirl/kuru/releases/assets/{}",index+1),"digest":if data.scenario.api_digest && target.triple==TARGET {Some(format!("sha256:{}",data.digest))} else {None}})
    }).collect();
    assets.push(json!({"name":"SHA256SUMS","browser_download_url":format!("https://github.com/replygirl/kuru/releases/download/v{VERSION}/SHA256SUMS"),"url":"https://api.github.com/repos/replygirl/kuru/releases/assets/6"}));
    json!({"tag_name":format!("v{VERSION}"),"draft":false,"prerelease":false,"created_at":"2020-01-01T00:00:00Z","published_at":"2020-01-01T00:00:00Z","assets":assets})
}

async fn serve(
    State(shared): State<Arc<Mutex<ServerData>>>,
    request: Request<Body>,
) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query();
    let mut data = shared.lock().unwrap();
    let mut status = StatusCode::OK;
    let mut content_type = "application/json";
    let mut rejected = None;
    let bytes = if request.headers().contains_key("authorization")
        || request.headers().contains_key("cookie")
    {
        rejected = Some("unexpected authentication header".to_owned());
        Vec::new()
    } else if method == Method::GET && path == "/mise-version" && query.is_none() {
        // `mise --version` also checks its own latest version outside CI. Our
        // intentionally cleared environment has no CI flag or version cache.
        // Keep that exact notification request local without disabling release
        // provenance, changing backend behavior, or accepting arbitrary traffic.
        content_type = "text/plain";
        b"2026.9.4\n".to_vec()
    } else if method == Method::GET
        && path == "/api/repos/replygirl/kuru/releases"
        && matches!(query, None | Some("per_page=100"))
    {
        serde_json::to_vec(&vec![release(&data)]).unwrap()
    } else if method == Method::GET
        && (path == format!("/api/repos/replygirl/kuru/releases/tags/v{VERSION}")
            || path == "/api/repos/replygirl/kuru/releases/latest")
        && query.is_none()
    {
        serde_json::to_vec(&release(&data)).unwrap()
    } else if method == Method::GET
        && path == format!("/api/repos/replygirl/kuru/releases/tags/{VERSION}")
        && query.is_none()
    {
        status = StatusCode::NOT_FOUND;
        br#"{"message":"Not Found"}"#.to_vec()
    } else if method == Method::GET
        && path
            == format!(
                "/api/repos/replygirl/kuru/attestations/sha256:{}",
                data.digest
            )
        && matches!(query, None | Some("per_page=30"))
    {
        br#"{"attestations":[]}"#.to_vec()
    } else if (path == format!("/download/v{VERSION}/{}", asset_name())
        || path == "/api/repos/replygirl/kuru/releases/assets/5")
        && query.is_none()
        && (method == Method::GET || method == Method::HEAD)
        && !data.scenario.missing
    {
        content_type = "application/zip";
        if data.scenario.api_fallback && method == Method::HEAD && path.starts_with("/download/") {
            status = StatusCode::NOT_FOUND;
            Vec::new()
        } else {
            let mut bytes = data.archive.clone();
            if data.scenario.corrupt {
                bytes[0] ^= 1;
            }
            bytes
        }
    } else if (path == format!("/download/v{VERSION}/SHA256SUMS")
        || path == "/api/repos/replygirl/kuru/releases/assets/6")
        && query.is_none()
        && (method == Method::GET || method == Method::HEAD)
    {
        content_type = "text/plain";
        let hash = if data.scenario.bad_checksum {
            "0".repeat(64)
        } else {
            data.digest.clone()
        };
        targets::CATALOG
            .iter()
            .map(|target| {
                format!(
                    "{hash}  {}\n",
                    archive::archive_name(VERSION, target.triple).unwrap()
                )
            })
            .collect::<String>()
            .into_bytes()
    } else {
        rejected = Some(format!("unexpected request {method} {}", request.uri()));
        Vec::new()
    };
    if let Some(reason) = rejected {
        data.rejected.push(reason);
        status = StatusCode::BAD_REQUEST;
    }
    data.requests.push(Traffic {
        method: method.clone(),
        path,
        bytes: if method == Method::HEAD {
            0
        } else {
            bytes.len()
        },
    });
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("content-length", bytes.len())
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .unwrap()
}

impl Server {
    async fn new(bytes: &[u8], scenario: Scenario) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(ServerData {
            scenario,
            archive: bytes.to_vec(),
            digest: archive::digest(bytes),
            requests: Vec::new(),
            rejected: Vec::new(),
        }));
        let router = Router::new().fallback(any(serve)).with_state(state.clone());
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = receive.await;
                })
                .await
        });
        Ok(Self {
            base,
            state,
            shutdown: Some(shutdown),
            task,
        })
    }
    async fn close(mut self) -> Result<()> {
        self.shutdown
            .take()
            .unwrap()
            .send(())
            .map_err(|_| anyhow::anyhow!("fixture server exited early"))?;
        tokio::time::timeout(Duration::from_secs(5), &mut self.task).await???;
        let rejected = self.state.lock().unwrap().rejected.clone();
        ensure!(
            rejected.is_empty(),
            "fixture rejected unexpected traffic: {rejected:?}"
        );
        Ok(())
    }
}

struct Installation {
    root: PathBuf,
    project: PathBuf,
    mise: PathBuf,
    env: Vec<(OsString, OsString)>,
    kuru_data: PathBuf,
    engine_cache: PathBuf,
    memory_cleanup: MiseMemoryCleanup,
}

struct MiseMemoryCleanup {
    root: Option<tempfile::TempDir>,
    options: Option<kuru_memory::OpenOptions>,
}

impl MiseMemoryCleanup {
    fn new(root: tempfile::TempDir) -> Self {
        Self {
            root: Some(root),
            options: None,
        }
    }

    fn arm(&mut self, options: kuru_memory::OpenOptions) {
        assert!(self.options.is_none(), "mise memory cleanup already armed");
        self.options = Some(options);
    }

    fn finish_after_installed_purge(&mut self) -> Result<()> {
        self.options
            .take()
            .context("native mise purge had no armed project owner")?;
        let root = self
            .root
            .take()
            .context("native mise fixture root is absent")?;
        let path = root.path().to_owned();
        root.close()
            .context("remove retired native mise fixture root")?;
        ensure!(!path.exists(), "retired native mise fixture root remains");
        Ok(())
    }

    async fn retain_after_error(&mut self) -> (PathBuf, Option<anyhow::Error>) {
        let retirement = if let Some(options) = self.options.take() {
            kuru_memory::test_support::retire_idle_service(&options)
                .await
                .err()
        } else {
            None
        };
        (self.retain(), retirement)
    }

    fn retire_blocking(&mut self) -> Result<()> {
        let Some(options) = self.options.take() else {
            return Ok(());
        };
        std::thread::Builder::new()
            .name("kuru-mise-memory-cleanup".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create native mise cleanup runtime")?;
                runtime.block_on(async move {
                    kuru_memory::test_support::retire_idle_service(&options)
                        .await
                        .context("retire native mise fixture memory owner")
                })
            })?
            .join()
            .map_err(|_| anyhow::anyhow!("native mise memory cleanup thread panicked"))?
    }

    fn retain(&mut self) -> PathBuf {
        self.root
            .take()
            .expect("native mise fixture root retained before cleanup")
            .keep()
    }

    fn retain_if_owned(&mut self) -> Option<PathBuf> {
        self.root.take().map(tempfile::TempDir::keep)
    }
}

impl Drop for MiseMemoryCleanup {
    fn drop(&mut self) {
        let abandoned = self.options.is_some();
        let panicking = std::thread::panicking();
        let retirement = self.retire_blocking();
        let retained = if abandoned || panicking || retirement.is_err() {
            self.retain_if_owned()
        } else {
            None
        };
        if panicking {
            let retained = retained
                .as_ref()
                .map_or_else(|| "already retained".to_owned(), |path| format!("{path:?}"));
            if let Err(error) = retirement {
                eprintln!(
                    "native mise memory cleanup failed while preserving the original panic; fixture root retained in place at {retained}: {error:#}"
                );
            } else {
                eprintln!(
                    "native mise fixture failed; fixture root retained in place at {retained}"
                );
            }
        } else if let Err(error) = retirement {
            let retained = retained
                .as_ref()
                .map_or_else(|| "already retained".to_owned(), |path| format!("{path:?}"));
            panic!(
                "native mise memory cleanup failed; fixture root retained in place at {retained}: {error:#}"
            );
        } else if abandoned {
            let retained = retained
                .as_ref()
                .map_or_else(|| "already retained".to_owned(), |path| format!("{path:?}"));
            eprintln!(
                "native mise conversation was cancelled; authenticated memory retirement completed and fixture root was retained in place at {retained}"
            );
        }
    }
}

impl Installation {
    fn new(mise: &Path, base: &str) -> Result<Self> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().to_owned();
        let project = root.join("project");
        fs::create_dir(&project)?;
        let mut env = mise_isolation::prepare(&root, &project)?;
        // This affects diagnostics only; it does not change installation policy.
        if let Some(path) = std::env::var_os("LLVM_PROFILE_FILE") {
            env.push(("LLVM_PROFILE_FILE".into(), path));
        }
        fs::write(
            project.join("mise.toml"),
            CONFIG.replace("FIXTURE_BASE", base),
        )?;
        let engine_cache = root.join("cold-engine-cache");
        fs::create_dir(root.join("appdata/kuru"))?;
        let config = std::collections::BTreeMap::from([(
            "memory",
            kuru_core::MemoryConfig {
                offline: true,
                cache_dir: Some(engine_cache.clone()),
                ..Default::default()
            },
        )]);
        fs::write(
            root.join("appdata/kuru/config.toml"),
            toml::to_string(&config)?,
        )?;
        let kuru_data = root.join("cold-kuru-data");
        Ok(Self {
            root,
            project,
            mise: mise.to_owned(),
            env,
            kuru_data,
            engine_cache,
            memory_cleanup: MiseMemoryCleanup::new(temporary),
        })
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.mise);
        command.fixture_allow_independent_service();
        command.env_clear().current_dir(&self.project);
        for (name, value) in &self.env {
            command.env(name, value);
        }
        command
    }
    async fn output(&self, args: &[&str]) -> Result<Output> {
        command::output(self.command().args(args), DEADLINE)
            .await
            .context("native mise command")
    }
    async fn success(&self, args: &[&str]) -> Result<String> {
        let output = self.output(args).await?;
        ensure!(
            output.status.success(),
            "mise {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }
    async fn isolation(&self) -> Result<()> {
        let paths: Value = serde_json::from_str(&self.success(&["config", "ls", "--json"]).await?)?;
        // Pin the actual client output schema; an unrecognized schema fails
        // instead of silently accepting unknown config discovery.
        let entries = paths
            .as_array()
            .context("mise config ls did not return an array")?;
        for entry in entries {
            let path = entry["path"]
                .as_str()
                .context("mise config entry lacks path")?;
            ensure!(
                checked_file_within(Path::new(path), &self.root)?,
                "mise discovered config outside fixture: {path}"
            );
        }
        for (setting, expected) in [
            ("use_versions_host", "false"),
            ("netrc", "false"),
            ("github.gh_cli_tokens", "false"),
            ("github.use_git_credentials", "false"),
        ] {
            ensure!(
                self.success(&["settings", "get", setting]).await?.trim() == expected,
                "unexpected effective mise setting {setting}"
            );
        }
        ensure!(
            self.success(&["settings", "get", "url_replacements"])
                .await?
                .contains("127.0.0.1"),
            "missing fixture routing"
        );
        Ok(())
    }
    async fn kuru(&self, args: &[&str]) -> Result<Value> {
        let mut command = self.command();
        command
            .args(["exec", "--", "kuru", "-C"])
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.kuru_data)
            .args(["--provider", "demo", "--mode", "freudian", "--no-dream"])
            .args(args);
        let output = command::output(&mut command, DEADLINE).await?;
        ensure!(
            output.status.success(),
            "installed Kuru {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    async fn purge_with_installed_kuru(&self) -> Result<()> {
        let mut command = self.command();
        command
            .args(["exec", "--", "kuru", "-C"])
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.kuru_data)
            .args(["memory", "purge", "--yes"]);
        let output = command::output(&mut command, DEADLINE).await?;
        ensure!(
            output.status.success(),
            "installed Kuru refused isolated memory purge: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let outcome: Value = serde_json::from_slice(&output.stdout)?;
        let scope = kuru_runtime::project_scope(&self.project)?;
        ensure!(
            outcome["project"] == scope
                && outcome["removed_trees"]
                    .as_u64()
                    .is_some_and(|count| count >= 1)
                && !kuru_memory::MemoryStore::exists(&self.kuru_data, &scope)?,
            "installed Kuru did not remove the exact isolated project memory"
        );
        Ok(())
    }
    fn memory_options(&self, binary: &Path, scope: String) -> kuru_memory::OpenOptions {
        let mut options = kuru_memory::OpenOptions::new(self.kuru_data.clone(), scope);
        options.config = kuru_core::MemoryConfig {
            offline: true,
            cache_dir: Some(self.engine_cache.clone()),
            ..Default::default()
        };
        options.supervisor = Some(binary.to_owned());
        options
    }

    async fn conversation(&mut self, binary: &Path) -> Result<()> {
        ensure!(
            !self.engine_cache.exists() && !self.kuru_data.exists(),
            "mise runtime fixture is not cold"
        );
        let scope = kuru_runtime::project_scope(&self.project)?;
        self.memory_cleanup
            .arm(self.memory_options(binary, scope.clone()));
        let result = async {
            self.conversation_inner(binary, &scope).await?;
            self.purge_with_installed_kuru().await
        }
        .await;
        match result {
            Ok(()) => self.memory_cleanup.finish_after_installed_purge(),
            Err(error) => {
                let (retained, retirement) = self.memory_cleanup.retain_after_error().await;
                let cleanup = retirement.map_or_else(
                    || "authenticated memory retirement completed".to_owned(),
                    |cleanup| format!("authenticated memory retirement also failed: {cleanup:#}"),
                );
                Err(error.context(format!(
                    "native mise conversation failed; {cleanup}; fixture root retained in place at {retained:?}"
                )))
            }
        }
    }

    async fn conversation_inner(&self, binary: &Path, scope: &str) -> Result<()> {
        let marker = "mise native cold marker violet-837";
        let first = self.kuru(&["run", marker, "--json"]).await?;
        let session = first["session"].as_str().context("first session ID")?;
        let before = self.kuru(&["memory", "status"]).await?;
        let second = self
            .kuru(&[
                "--resume",
                session,
                "run",
                "Resume that exact saved conversation",
                "--json",
            ])
            .await?;
        ensure!(
            second["session"] == session,
            "mise executable did not resume the durable session"
        );
        let after = self.kuru(&["memory", "status"]).await?;
        ensure!(
            before["engine"] == "dolt" && before["revision"] != after["revision"],
            "Dolt revisions did not persist across reopen"
        );
        let sessions = self.kuru(&["sessions"]).await?;
        ensure!(
            sessions
                .as_array()
                .context("session array")?
                .iter()
                .any(|entry| entry["id"] == session && entry["turns"] == 2),
            "session turn count did not persist"
        );
        let history = self.kuru(&["memory", "history", "--limit", "100"]).await?;
        for revision in [&before["revision"], &after["revision"]] {
            ensure!(
                history
                    .as_array()
                    .context("history array")?
                    .iter()
                    .any(|entry| entry["hash"] == *revision),
                "conversation revision absent from history"
            );
        }
        let mut options = self.memory_options(binary, scope.to_owned());
        options.read_only = true;
        let store = kuru_memory::MemoryStore::open(options).await?;
        let transcript = store
            .history(&format!("{scope}/transcript/{session}"), 100)
            .await;
        let close = store.close().await;
        let transcript = transcript?;
        close?;
        ensure!(
            transcript
                .iter()
                .any(|message| { message.role == "user" && message.plain_text() == Some(marker) }),
            "mise-installed transcript lost original input"
        );
        ensure!(
            transcript
                .iter()
                .filter(|message| message.role == "assistant")
                .count()
                == 2,
            "mise-installed transcript duplicated or lost an answer"
        );
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../kuru-memory/support/dolt-assets.json"
        ))?;
        let asset = manifest["assets"]
            .as_array()
            .context("engine catalog")?
            .iter()
            .find(|asset| asset["target"] == TARGET)
            .context("Windows engine")?;
        let engine = self
            .engine_cache
            .join(manifest["version"].as_str().context("engine version")?)
            .join(TARGET);
        for (name, size, hash) in [
            ("dolt.exe", "executable_bytes", "executable_sha256"),
            ("LICENSES", "license_bytes", "license_sha256"),
        ] {
            let bytes = fs::read(engine.join(name))?;
            ensure!(
                bytes.len() as u64 == asset[size].as_u64().context("engine size")?
                    && archive::digest(&bytes) == asset[hash].as_str().context("engine digest")?,
                "mise-installed engine {name} differs from embedded manifest"
            );
        }
        Ok(())
    }
}

const CONFIG: &str = r#"[settings]
use_versions_host=false
use_versions_host_track=false
netrc=false
[settings.github]
gh_cli_tokens=false
use_git_credentials=false
credential_command=""
oauth_client_id=""
[settings.url_replacements]
'regex:^https://api\.github\.com/?$'='FIXTURE_BASE/api'
'regex:^https://api\.github\.com/repos/replygirl/kuru(/|$)'='FIXTURE_BASE/api/repos/replygirl/kuru$1'
'regex:^https://github\.com/replygirl/kuru/releases/download/'='FIXTURE_BASE/download/'
'regex:^https://mise\.jdx\.dev/VERSION$'='FIXTURE_BASE/mise-version'
'regex:^(https?://.*)'='FIXTURE_BASE/unexpected/$1'
"#;

async fn run_archive(
    bytes: Vec<u8>,
    expected: String,
    support: Option<&kuru_delivery::shell_support::Files>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let env = ["PATH", "PATHEXT", "SystemRoot"]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key.into(), value)))
        .collect();
    let mise = configured_command(std::ffi::OsStr::new("mise"), &[], &cwd, env)?.executable;
    for (index, scenario) in [
        Scenario::VALID,
        Scenario {
            corrupt: true,
            ..Scenario::VALID
        },
        Scenario {
            api_digest: false,
            ..Scenario::VALID
        },
        Scenario {
            api_digest: false,
            bad_checksum: true,
            ..Scenario::VALID
        },
        Scenario {
            api_fallback: true,
            ..Scenario::VALID
        },
        Scenario {
            missing: true,
            ..Scenario::VALID
        },
    ]
    .into_iter()
    .enumerate()
    {
        let server = Server::new(&bytes, scenario).await?;
        let mut installed = Installation::new(&mise, &server.base)?;
        ensure!(
            installed
                .success(&["--version"])
                .await?
                .contains("2026.9.4"),
            "unexpected mise version"
        );
        ensure!(
            server
                .state
                .lock()
                .unwrap()
                .requests
                .iter()
                .any(|request| request.method == Method::GET && request.path == "/mise-version"),
            "the real mise version notification did not reach its isolated route"
        );
        if index == 0 {
            installed.isolation().await?;
            ensure!(
                installed
                    .success(&["ls-remote", "github:replygirl/kuru"])
                    .await?
                    .lines()
                    .any(|line| line == VERSION),
                "fixture release was not listed"
            );
        }
        let selection = format!("github:replygirl/kuru@{VERSION}");
        let result = installed.output(&["use", &selection]).await?;
        let should_fail = scenario.corrupt || scenario.bad_checksum || scenario.missing;
        if should_fail {
            ensure!(
                !result.status.success(),
                "mise accepted corrupt or missing candidate {index}"
            );
            ensure!(
                !installed.output(&["which", "kuru"]).await?.status.success(),
                "failed candidate became selectable"
            );
        } else {
            ensure!(
                result.status.success(),
                "mise scenario{index} failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let path = PathBuf::from(installed.success(&["which", "kuru"]).await?.trim());
            ensure!(
                checked_file_within(&path, &installed.root.join("mise-data/installs"))?,
                "mise selected a binary outside its isolated install root"
            );
            ensure!(
                archive::digest(&fs::read(&path)?) == expected,
                "mise selected incorrect executable bytes"
            );
            if index == 0 {
                // Reproduce the isolated-environment failure independently of
                // archive installation: the pinned resolver has no default
                // executable extensions when PATHEXT is absent.
                let mut missing_extensions = installed.command();
                missing_extensions
                    .env_remove("PATHEXT")
                    .args(["exec", "--", "kuru", "--version"]);
                let rejected = command::output(&mut missing_extensions, DEADLINE).await?;
                ensure!(
                    !rejected.status.success()
                        && String::from_utf8_lossy(&rejected.stderr)
                            .contains("cannot find binary path"),
                    "missing-PATHEXT control did not reproduce native lookup rejection: {}",
                    String::from_utf8_lossy(&rejected.stderr)
                );
            }
            ensure!(
                installed
                    .success(&["exec", "--", "kuru", "--version"])
                    .await?
                    .trim()
                    == format!("kuru {VERSION}"),
                "mise activation selected wrong version"
            );
            if index == 0 {
                if let Some(files) = support {
                    for (name, shell) in [
                        ("completions/kuru.bash", "bash"),
                        ("completions/_kuru", "zsh"),
                        ("completions/kuru.fish", "fish"),
                        ("completions/kuru.ps1", "powershell"),
                    ] {
                        let output = installed
                            .success(&["exec", "--", "kuru", "completions", shell])
                            .await?;
                        ensure!(
                            files.get(name) == Some(output.as_bytes()),
                            "staged support differs from the installed CLI"
                        );
                    }
                    let man = installed.success(&["exec", "--", "kuru", "man"]).await?;
                    ensure!(
                        files.get("man/kuru.1") == Some(man.as_bytes()),
                        "staged manual differs from the installed CLI"
                    );
                }
                installed.conversation(&path).await?;
            }
        }
        {
            let traffic = server.state.lock().unwrap();
            ensure!(
                traffic.rejected.is_empty(),
                "unexpected native mise traffic: {:?}",
                traffic.rejected
            );
            if !scenario.missing {
                ensure!(
                    traffic
                        .requests
                        .iter()
                        .any(|request| request.method == Method::GET
                            && request.bytes == bytes.len()
                            && (request.path.ends_with(&asset_name())
                                || request.path.ends_with("assets/5"))),
                    "mise never downloaded the full Windows ZIP"
                );
            }
            if !scenario.api_digest {
                ensure!(
                    traffic
                        .requests
                        .iter()
                        .any(|request| request.method == Method::GET
                            && (request.path.ends_with("SHA256SUMS")
                                || request.path.ends_with("assets/6"))),
                    "mise skipped release checksum file"
                );
            }
            if scenario.api_fallback {
                ensure!(
                    traffic
                        .requests
                        .iter()
                        .any(|request| request.method == Method::GET
                            && request.path.ends_with("assets/5")),
                    "API fallback was not exercised"
                );
            }
            println!(
                "native mise fixture scenario{index}: expected_failure={should_fail} requests={} actual_archive_sha256={} actual_executable_sha256={expected}",
                traffic.requests.len(),
                traffic.digest
            );
        }
        server.close().await?;
    }
    Ok(())
}

pub async fn run(binary: &Path) -> Result<()> {
    ensure!(
        binary.is_absolute(),
        "mise acceptance requires an absolute actual Kuru binary"
    );
    let root = tempfile::tempdir()?;
    let archive = archive::package(binary, TARGET, VERSION, root.path())?;
    let bytes = fs::read(archive)?;
    let expected = archive::digest(&fs::read(binary)?);
    run_archive(bytes, expected, None).await
}

pub async fn run_staged(archive_path: &Path) -> Result<()> {
    ensure!(
        archive_path.is_absolute(),
        "staged mise acceptance requires an absolute release archive"
    );
    ensure!(
        archive_path.file_name().and_then(|name| name.to_str()) == Some(&asset_name()),
        "staged mise acceptance requires the exact versioned Windows ZIP"
    );
    let directory = archive_path
        .parent()
        .and_then(Path::to_str)
        .context("staged Windows archive directory is not Unicode")?;
    let manifest = archive::read_asset(directory, "SHA256SUMS", 64 * 1024).await?;
    let expected_archive = archive::expected_digest(&manifest, &asset_name())?;
    let bytes = archive::read_asset(directory, &asset_name(), archive::MAX_ARCHIVE_BYTES).await?;
    ensure!(
        archive::digest(&bytes) == expected_archive,
        "staged Windows archive differs from SHA256SUMS"
    );
    let (executable, support) = archive::verified_release(directory, VERSION, TARGET).await?;
    ensure!(
        support.is_some(),
        "staged new release is missing its paired shell support"
    );
    run_archive(bytes, archive::digest(&executable), support.as_ref()).await?;
    // Upgrade compatibility is required only from the immediately previous
    // published release, resolved now rather than pinned.
    let previous = published_windows::previous_windows_release(VERSION).await?;
    previous_updater_accepts_staged_release(
        &previous,
        directory,
        &executable,
        support.as_ref().unwrap(),
    )
    .await
}

async fn previous_updater_accepts_staged_release(
    previous: &published_windows::PreviousWindowsRelease,
    staged_directory: &str,
    staged_executable: &[u8],
    staged_support: &shell_support::Files,
) -> Result<()> {
    let old_version = previous.version.as_str();
    let root = tempfile::tempdir()?;
    let old_release_dir = root.path().join("verified-old-release");
    fs::create_dir(&old_release_dir)?;
    // Already authenticated against its own SHA256SUMS and GitHub's digests;
    // the installer path verifies these local copies again before extraction.
    fs::write(old_release_dir.join("SHA256SUMS"), &previous.manifest)?;
    fs::write(
        old_release_dir.join(archive::archive_name(old_version, TARGET)?),
        &previous.archive,
    )?;
    let old_directory = old_release_dir
        .to_str()
        .context("isolated old release directory is not Unicode")?;
    let (old_executable, old_support) =
        archive::verified_release(old_directory, old_version, TARGET).await?;

    let installation = root.path().join("old-installed-bin");
    let project = root.path().join("project");
    fs::create_dir(&installation)?;
    fs::create_dir(&project)?;
    let installed = installation.join("kuru.exe");
    fs::write(&installed, &old_executable)?;
    let environment = mise_isolation::prepare(root.path(), &project)?;
    let command = || {
        let mut command = Command::new(&installed);
        command.env_clear().current_dir(&project);
        for (name, value) in &environment {
            command.env(name, value);
        }
        command
    };
    let mut version_probe = command();
    version_probe.arg("--version");
    let output = command::output(&mut version_probe, DEADLINE).await?;
    ensure!(
        output.status.success()
            && output.stdout.as_slice() == format!("kuru {old_version}\n").as_bytes(),
        "verified old executable did not identify as v{old_version}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut update = command();
    update
        .args(["update", "--version", VERSION, "--release-base"])
        .arg(staged_directory);
    let output = command::output(&mut update, DEADLINE).await?;
    ensure!(
        output.status.success(),
        "actual v{old_version} updater rejected the staged release: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        fs::read(&installed)? == staged_executable,
        "old updater did not install the exact verified staged executable"
    );
    // A marked previous core shipped with the support-aware updater, which
    // stages the candidate's versioned snapshot before replacing itself.
    let managed = installation.join("share");
    if old_support.is_some() {
        ensure!(
            shell_support::read_generated(&managed.join("kuru").join(VERSION).join(TARGET))?
                == *staged_support,
            "support-aware v{old_version} updater did not install the exact staged support"
        );
    } else {
        ensure!(
            !managed.exists(),
            "executable-only v{old_version} updater unexpectedly claimed managed shell support"
        );
    }
    let mut new_version = command();
    new_version.arg("--version");
    let output = command::output(&mut new_version, DEADLINE).await?;
    ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim() == format!("kuru {VERSION}"),
        "upgraded executable did not report the staged version"
    );

    // Not a repair of anything broken: this proves the upgraded binary's own
    // regeneration matches the staged sidecar byte-for-byte, independent of
    // whatever the old updater itself did or did not install as support.
    let regenerated = root.path().join("regenerated-support-sidecar");
    for (name, args) in [
        ("completions/kuru.bash", &["completions", "bash"][..]),
        ("completions/_kuru", &["completions", "zsh"][..]),
        ("completions/kuru.fish", &["completions", "fish"][..]),
        ("completions/kuru.ps1", &["completions", "powershell"][..]),
        ("man/kuru.1", &["man"][..]),
    ] {
        let mut generator = command();
        generator.args(args);
        let output = command::output(&mut generator, DEADLINE).await?;
        ensure!(
            output.status.success() && staged_support.get(name) == Some(output.stdout.as_slice()),
            "upgraded executable did not regenerate exact staged support {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let destination = regenerated.join(name);
        fs::create_dir_all(
            destination
                .parent()
                .context("regenerated support member has no parent")?,
        )?;
        fs::write(&destination, &output.stdout)?;
        ensure!(
            fs::read(&destination)? == output.stdout,
            "regeneration matching the staged sidecar changed on disk for {name}"
        );
    }
    ensure!(
        !root.path().join("xdg-data/kuru").exists() && !root.path().join("appdata/kuru").exists(),
        "old update or support sidecar regeneration created application private data"
    );
    root.close()
        .context("retire isolated old-updater acceptance root")
}
