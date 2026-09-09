#![cfg(feature = "tooling")]
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_delivery::archive::digest;
use kuru_delivery::release::{self, GitHub, Version};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
fn v(input: &str) -> Version {
    input.parse().unwrap()
}
struct Repo {
    temp: TempDir,
}
impl Repo {
    async fn new() -> Self {
        let this = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        let root = this.root();
        release::git(root, &["init", "-b", "main"]).await.unwrap();
        for (key, value) in [
            ("user.name", "Release fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            release::git(root, &["config", key, value]).await.unwrap();
        }
        fs::create_dir(root.join("app")).unwrap();
        fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname = \"kuru\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 4\n\n[[package]]\nname = \"kuru\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"unchanged\"\n").unwrap();
        fs::write(root.join("cog.toml"), include_str!("../../../cog.toml")).unwrap();
        fs::write(
            root.join("communique.toml"),
            include_str!("../../../communique.toml"),
        )
        .unwrap();
        this.commit("feat: initial peer harness").await;
        this
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    async fn commit(&self, message: &str) -> String {
        release::git(self.root(), &["add", "."]).await.unwrap();
        release::git(self.root(), &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
        self.head().await
    }
    async fn head(&self) -> String {
        release::git(self.root(), &["rev-parse", "HEAD"])
            .await
            .unwrap()
    }
    async fn tag(&self, tag: &str) {
        release::git(self.root(), &["tag", tag]).await.unwrap();
    }
}
#[tokio::test]
async fn actual_cog_conventional_history_and_noop_are_read_only() {
    let repo = Repo::new().await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.0")
    );
    repo.tag("v0.1.0").await;
    assert!(
        release::compute_version(repo.root(), "auto")
            .await
            .unwrap_err()
            .to_string()
            .contains("no commits")
    );
    repo.commit("fix: preserve isolated memories").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.1.1")
    );
    repo.commit("feat: add peer relationships").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    repo.commit("feat!: replace configuration format").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("0.2.0")
    );
    for (kind, expected) in [("major", "1.0.0"), ("minor", "0.2.0"), ("patch", "0.1.1")] {
        assert_eq!(
            release::compute_version(repo.root(), kind).await.unwrap(),
            v(expected)
        );
    }
    repo.tag("v1.0.0").await;
    repo.commit("fix!: remove legacy configuration").await;
    assert_eq!(
        release::compute_version(repo.root(), "auto").await.unwrap(),
        v("2.0.0")
    );
    assert_eq!(
        release::git(repo.root(), &["status", "--porcelain"])
            .await
            .unwrap(),
        ""
    );
    assert!(
        release::compute_version(repo.root(), "--help")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn stamps_only_workspace_versions_and_rejects_inconsistent_locks() {
    let repo = Repo::new().await;
    let path = repo.root().join("Cargo.lock");
    let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(release::stamp(repo.root(), v("0.2.0")).unwrap().len(), 2);
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let after: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(before["package"][1], after["package"][1]);
    assert_eq!(after["package"][0]["version"].as_str(), Some("0.2.0"));
    assert!(release::stamp(repo.root(), v("0.2.0")).unwrap().is_empty());
    let manifest = fs::read(repo.root().join("Cargo.toml")).unwrap();
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("version = \"0.2.0\"", "version = \"0.0.1\""),
    )
    .unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("inconsistent")
    );
    assert_eq!(fs::read(repo.root().join("Cargo.toml")).unwrap(), manifest);
    fs::write(path, "version = 4\n").unwrap();
    assert!(
        release::stamp(repo.root(), v("0.3.0"))
            .unwrap_err()
            .to_string()
            .contains("every workspace")
    );
}
#[tokio::test]
async fn plans_initial_release_and_explicit_resume_without_moving_refs() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let initial = release::plan(repo.root(), "auto", "", "").await.unwrap();
    assert_eq!(initial.base_sha, head);
    assert!(!initial.resume);
    assert!(
        release::plan(repo.root(), "auto", "0.1.0", &head)
            .await
            .unwrap()
            .resume
    );
    assert!(
        release::plan(repo.root(), "auto", "0.1.0", "")
            .await
            .is_err()
    );
    assert!(
        release::plan(repo.root(), "auto", "0.2.0", &head)
            .await
            .is_err()
    );
    assert!(release::plan(repo.root(), "patch", "", "").await.is_err());
    assert!(
        release::plan(repo.root(), "auto", "0.1.0", A)
            .await
            .is_err()
    );
    for invalid in ["1.2", "01.2.3", "1.0.0\nx=y", "1.0.0;id", "1.0.0-rc.1", ""] {
        assert!(invalid.parse::<Version>().is_err());
    }
    for invalid in ["HEAD", "abcd", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        assert!(release::checked_sha(invalid).is_err());
    }
}

#[tokio::test]
async fn real_cli_calculates_plans_and_stamps_without_remote_credentials() {
    let repo = Repo::new().await;
    let binary = env!("CARGO_BIN_EXE_kuru-release");
    let output = tokio::process::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["version", "auto"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "0.1.0");
    let outputs = repo.root().join("outputs");
    let output = tokio::process::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["plan", "--bump", "auto"])
        .env("GITHUB_OUTPUT", &outputs)
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert!(
        fs::read_to_string(outputs)
            .unwrap()
            .contains("version=0.1.0\n")
    );
    let output = tokio::process::Command::new(binary)
        .arg("--root")
        .arg(repo.root())
        .args(["stamp", "0.2.0"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        release::workspace_version(repo.root(), None).await.unwrap(),
        v("0.2.0")
    );
    let output = tokio::process::Command::new(binary)
        .args(["commit", "--version", "0.2.0", "--expected-sha", A])
        .env_remove("GH_REPO")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_REPO is required"));
    let output = tokio::process::Command::new(binary)
        .args([
            "publish",
            "--version",
            "0.2.0",
            "--sha",
            A,
            "--notes",
            "missing",
        ])
        .env("GH_REPO", "fixture/kuru")
        .env_remove("GH_TOKEN")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("GH_TOKEN is required"));
    let output = tokio::process::Command::new(binary)
        .args(["stamp", "1.0.0;id"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
}

#[derive(Default)]
struct Remote {
    head: String,
    tag: Option<Value>,
    tag_object: Option<Value>,
    release: Option<Value>,
    calls: Vec<(Method, String, Value)>,
    uploads: Vec<String>,
    fail_upload: Option<usize>,
    corrupt_upload: bool,
    malformed: bool,
}
struct Server {
    api: GitHub,
    state: Arc<Mutex<Remote>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(head: &str) -> Self {
        let state = Arc::new(Mutex::new(Remote {
            head: head.into(),
            ..Remote::default()
        }));
        let app = Router::new().fallback(handler).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            api: GitHub::with_endpoints("fixture/kuru", "fixture-token", &endpoint, &endpoint)
                .unwrap(),
            state,
            task,
        }
    }
}
fn response(status: StatusCode, value: Value) -> Response {
    (status, axum::Json(value)).into_response()
}
async fn handler(State(shared): State<Arc<Mutex<Remote>>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-token"
    );
    let bytes = to_bytes(request.into_body(), 300 * 1024 * 1024)
        .await
        .unwrap();
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut state = shared.lock().unwrap();
    state
        .calls
        .push((method.clone(), path.clone(), payload.clone()));
    if state.malformed {
        return (StatusCode::OK, "invalid-json").into_response();
    }
    let suffix = path.strip_prefix("/repos/fixture/kuru/").unwrap_or(&path);
    let value = match (method.clone(), suffix) {
        (Method::POST, "/graphql") => {
            if payload["variables"]["input"]["expectedHeadOid"].as_str() != Some(&state.head) {
                return response(
                    StatusCode::OK,
                    json!({"errors":[{"message":"expected head changed; private context"}]}),
                );
            }
            json!({"data":{"createCommitOnBranch":{"commit":{"oid":B}}}})
        }
        (Method::GET, "commits/main") => json!({"sha":state.head}),
        (Method::GET, value) if value.starts_with("git/ref/tags/") => {
            if let Some(tag) = &state.tag {
                json!({"object":tag})
            } else {
                return response(StatusCode::NOT_FOUND, json!({"message":"missing"}));
            }
        }
        (Method::POST, "git/tags") => {
            state.tag_object = Some(json!({"type":"commit","sha":payload["object"]}));
            json!({"sha":C})
        }
        (Method::POST, "git/refs") => {
            state.tag = Some(json!({"type":"tag","sha":payload["sha"]}));
            json!({"object":state.tag})
        }
        (Method::GET, value) if value.starts_with("git/tags/") => {
            json!({"object":state.tag_object})
        }
        (Method::GET, "releases") => state
            .release
            .as_ref()
            .map(|r| json!([r]))
            .unwrap_or(json!([])),
        (Method::POST, "releases") => {
            let mut value = payload;
            value["id"] = json!(17);
            value["assets"] = json!([]);
            value["html_url"] = json!("https://example.invalid/release");
            state.release = Some(value.clone());
            value
        }
        (Method::GET, "releases/17") => state.release.clone().unwrap(),
        (Method::POST, "releases/17/assets") => {
            if state.fail_upload == Some(state.uploads.len()) {
                return response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"message":"fixture interrupted"}),
                );
            }
            let name = url::form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "name")
                .unwrap()
                .1
                .into_owned();
            state.uploads.push(name.clone());
            let digest = if state.corrupt_upload {
                "0".repeat(64)
            } else {
                digest(&bytes)
            };
            let asset = json!({"name":name,"digest":format!("sha256:{digest}")});
            state.release.as_mut().unwrap()["assets"]
                .as_array_mut()
                .unwrap()
                .push(asset.clone());
            asset
        }
        (Method::PATCH, "releases/17") => {
            let release = state.release.as_mut().unwrap();
            release["draft"] = payload["draft"].clone();
            release.clone()
        }
        _ => {
            return response(
                StatusCode::BAD_REQUEST,
                json!({"message":"unexpected request"}),
            );
        }
    };
    response(StatusCode::OK, value)
}
#[tokio::test]
async fn signed_api_commit_uses_expected_head_and_only_scoped_payloads() {
    let repo = Repo::new().await;
    let head = repo.head().await;
    let server = Server::new(&head).await;
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.1.0"))
            .await
            .unwrap(),
        head
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    release::stamp(repo.root(), v("0.2.0")).unwrap();
    assert_eq!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap(),
        B
    );
    let payload = server
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .find(|(_, path, _)| path == "/graphql")
        .unwrap()
        .2
        .clone();
    let input = &payload["variables"]["input"];
    assert_eq!(input["expectedHeadOid"], head);
    assert_eq!(input["branch"]["branchName"], "main");
    assert_eq!(input["message"]["headline"], "chore(release): v0.2.0");
    let files = input["fileChanges"]["additions"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let manifest = files.iter().find(|f| f["path"] == "Cargo.toml").unwrap();
    let data: toml::Value = toml::from_str(
        &String::from_utf8(
            STANDARD
                .decode(manifest["contents"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        data["workspace"]["package"]["version"].as_str(),
        Some("0.2.0")
    );
    server.state.lock().unwrap().head = C.into();
    let error = release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected main head"));
    assert!(!error.contains("private context"));
    fs::write(repo.root().join("cog.toml"), "# unrelated change\n").unwrap();
    assert!(
        release::commit_version(repo.root(), &server.api, &head, v("0.2.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("unrelated")
    );
}
struct Archives {
    temp: TempDir,
    directory: PathBuf,
    notes: PathBuf,
}
impl Archives {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("dist");
        fs::create_dir(&directory).unwrap();
        // Publication validates opaque packager output. Installer/package tests
        // independently validate tar contents; here each native asset is distinct.
        for target in release::TARGETS {
            let name = format!("kuru-0.1.0-{target}.tar.gz");
            let data = format!("verified native fixture {target}");
            fs::write(directory.join(&name), &data).unwrap();
            fs::write(
                directory.join(format!("{name}.sha256")),
                format!("{}  {name}\n", digest(data.as_bytes())),
            )
            .unwrap();
        }
        let notes = temp.path().join("notes.md");
        fs::write(&notes, "# Kuru 0.1.0\n\nPersistent peer conversations.\n").unwrap();
        Self {
            temp,
            directory,
            notes,
        }
    }
    async fn publish(&self, api: &GitHub) -> anyhow::Result<String> {
        release::publish(api, &self.directory, v("0.1.0"), A, &self.notes).await
    }
}
#[tokio::test]
async fn interrupted_draft_resumes_missing_assets_then_publishes_exact_commit() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().fail_upload = Some(2);
    assert!(archives.publish(&server.api).await.is_err());
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.release.as_ref().unwrap()["draft"], true);
        assert!(
            remote
                .calls
                .iter()
                .all(|(method, _, _)| method != Method::PATCH)
        );
    }
    server.state.lock().unwrap().fail_upload = None;
    assert_eq!(
        archives.publish(&server.api).await.unwrap(),
        "https://example.invalid/release"
    );
    assert_eq!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap()
            .as_deref(),
        Some(A)
    );
    {
        let remote = server.state.lock().unwrap();
        assert_eq!(remote.uploads.len(), 5);
        assert_eq!(remote.release.as_ref().unwrap()["draft"], false);
        assert!(
            remote.release.as_ref().unwrap()["body"]
                .as_str()
                .unwrap()
                .contains(A)
        );
    }
    let snapshot = server.state.lock().unwrap().release.clone();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("already published")
    );
    assert_eq!(server.state.lock().unwrap().release, snapshot);
    assert_eq!(
        fs::read_to_string(archives.directory.join("SHA256SUMS"))
            .unwrap()
            .lines()
            .count(),
        4
    );
}
#[tokio::test]
async fn missing_corrupt_assets_and_empty_notes_prevent_all_remote_writes() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    let name = format!("kuru-0.1.0-{}.tar.gz", release::TARGETS[0]);
    let path = archives.directory.join(name);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("four supported")
    );
    fs::write(&path, b"corrupt").unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    fs::write(path, original).unwrap();
    fs::write(&archives.notes, "").unwrap();
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonempty")
    );
    assert!(server.state.lock().unwrap().calls.is_empty());
}
#[tokio::test]
async fn immutable_tag_conflicts_and_unowned_or_corrupt_drafts_are_rejected() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().tag = Some(json!({"type":"commit","sha":B}));
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("different commit")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method == Method::GET)
    );
    server.state.lock().unwrap().tag = None;
    server.state.lock().unwrap().fail_upload = Some(1);
    assert!(archives.publish(&server.api).await.is_err());
    server.state.lock().unwrap().release.as_mut().unwrap()["assets"][0]["digest"] = json!("bad");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    server.state.lock().unwrap().release.as_mut().unwrap()["body"] = json!("another draft");
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
}
#[tokio::test]
async fn bad_api_shapes_and_final_uploaded_digest_never_publish() {
    let archives = Archives::new();
    let server = Server::new(A).await;
    server.state.lock().unwrap().corrupt_upload = true;
    assert!(
        archives
            .publish(&server.api)
            .await
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(method, _, _)| method != Method::PATCH)
    );
    server.state.lock().unwrap().malformed = true;
    assert!(release::tag_commit(&server.api, v("0.1.0")).await.is_err());
    server.state.lock().unwrap().malformed = false;
    server.state.lock().unwrap().tag = Some(json!({"type":"tree","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    server.state.lock().unwrap().tag = Some(json!({"type":"tag","sha":C}));
    server.state.lock().unwrap().tag_object = Some(json!({"type":"tag","sha":C}));
    assert!(
        release::tag_commit(&server.api, v("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("resolve")
    );
    assert!(GitHub::new("bad/repo/path", "fixture").is_err());
    assert!(
        GitHub::with_endpoints(
            "fixture/kuru",
            "fixture",
            "http://example.invalid/",
            "http://example.invalid/"
        )
        .is_err()
    );
    assert!(GitHub::new("fixture/kuru", "").is_err());
    assert!(archives.temp.path().exists());
}
