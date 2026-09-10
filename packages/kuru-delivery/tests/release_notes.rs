#![cfg(feature = "tooling")]
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use kuru_delivery::{
    notes::{self, ProviderOptions},
    release::{self, Version},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;

const FRAMEWORKS: &str = "apps/kuru-docs/concepts/frameworks.md";
const COMMITTED_FRAMEWORKS: &str = "# Frameworks\nCommitted IFS default: seven persistent peers.\n";
const NOTES_BODY: &str = "Persistent peer conversations are now available.";
const NOTES_HEADING: &str = "# v0.1.0: Kuru 0.1.0\n\n";
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
fn version(input: &str) -> Version {
    input.parse().unwrap()
}
struct Fixture {
    root: TempDir,
}
impl Fixture {
    async fn new() -> Self {
        let this = Self {
            root: tempfile::tempdir().unwrap(),
        };
        let root = this.path();
        release::git(root, &["init", "-b", "main"]).await.unwrap();
        for (key, value) in [
            ("user.name", "Release fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            release::git(root, &["config", key, value]).await.unwrap();
        }
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("communique.toml"),
            include_str!("../../../communique.toml"),
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "/target/\n").unwrap();
        for (path, text) in [
            (FRAMEWORKS, COMMITTED_FRAMEWORKS),
            (
                "apps/kuru-docs/concepts/memory.md",
                "Private part memories.",
            ),
            (
                "apps/kuru-docs/concepts/sessions.md",
                "Persistent sessions.",
            ),
            (
                "apps/kuru-docs/guide/authentication.md",
                "Codex owns login.",
            ),
            (
                "apps/kuru-docs/reference/configuration.md",
                "Project preferences persist.",
            ),
            (
                "apps/kuru-docs/guide/first-conversation.md",
                "Quiet ambient motion.",
            ),
        ] {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        this.commit("feat: initial persistent peer harness").await;
        this
    }
    fn path(&self) -> &Path {
        self.root.path()
    }
    async fn commit(&self, message: &str) -> String {
        release::git(self.path(), &["add", "."]).await.unwrap();
        release::git(self.path(), &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
        self.head().await
    }
    async fn head(&self) -> String {
        release::git(self.path(), &["rev-parse", "HEAD"])
            .await
            .unwrap()
    }
    async fn generate(&self, api: &Api, output: &Path) -> anyhow::Result<()> {
        tokio::time::timeout(
            TEST_TIMEOUT,
            notes::generate(
                self.path(),
                &self.head().await,
                version("0.1.0"),
                output,
                &api.options(),
            ),
        )
        .await
        .expect("Communiqué fixture exceeded its deadline")
    }
    async fn assert_no_tag(&self) {
        assert_eq!(release::git(self.path(), &["tag"]).await.unwrap(), "");
    }
}
#[derive(Clone)]
enum Reply {
    Notes(String),
    Unauthorized,
    Malformed,
    NativeThinking,
}
#[derive(Clone, Debug)]
struct Call {
    path: String,
    authorized: bool,
    body: Value,
}
struct Api {
    endpoint: String,
    calls: Arc<Mutex<Vec<Call>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Api {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn tool_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "choices": [{
            "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{
                    "id": id, "type": "function",
                    "function": {"name": name, "arguments": arguments.to_string()}
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 10}
    })
}
impl Api {
    async fn new(reply: Reply) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(move |State(calls): State<Arc<Mutex<Vec<Call>>>>, request: Request| {
            let reply = reply.clone();
            async move {
                let path = request.uri().path().to_owned();
                let authorized = if matches!(reply, Reply::NativeThinking) {
                    request.headers().get("x-api-key").is_some_and(|value| value == "fixture-key")
                } else {
                    request.headers().get("authorization").is_some_and(|value| value == "Bearer fixture-key")
                };
                let body: Value = serde_json::from_slice(&to_bytes(request.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
                let turn = {
                    let mut calls = calls.lock().unwrap();
                    calls.push(Call { path, authorized, body });
                    calls.len()
                };
                let (status, value) = match reply {
                    Reply::Notes(_) if turn == 1 => (StatusCode::OK, tool_call("read-fixture", "read_file", json!({"path": FRAMEWORKS}))),
                    Reply::Notes(body) => (StatusCode::OK, tool_call("submit-fixture", "submit_release_notes", json!({
                        "changelog": "- Persistent peer conversations.",
                        "release_title": "Kuru 0.1.0", "release_body": body
                    }))),
                    Reply::Unauthorized => (StatusCode::UNAUTHORIZED, json!({"error": {"message": "sensitive provider payload: fixture-key"}})),
                    Reply::Malformed => (StatusCode::OK, json!({"unexpected": "provider payload"})),
                    Reply::NativeThinking => (StatusCode::OK, json!({
                        "content": [{"type": "thinking", "thinking": "", "signature": "fixture"}, {"type": "text", "text": "# Notes\nUnsupported response shape."}],
                        "stop_reason": "end_turn", "usage": {"input_tokens": 10, "output_tokens": 10}
                    })),
                };
                (status, axum::Json(value)).into_response()
            }
        }).with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            calls,
            task,
        }
    }
    fn options(&self) -> ProviderOptions {
        ProviderOptions {
            endpoint: Some(format!("{}/v1", self.endpoint)),
            api_key: Some("fixture-key".into()),
            omit_github_context: true,
        }
    }
}
#[tokio::test]
async fn initial_root_inventory_and_previous_release_are_in_supported_config() {
    let repo = Fixture::new().await;
    let head = repo.head().await;
    let (text, previous) = notes::configuration(repo.path(), &head, version("0.1.0"))
        .await
        .unwrap();
    let config: toml::Value = toml::from_str(&text).unwrap();
    assert!(previous.is_none());
    let context = config["context"].as_str().unwrap();
    assert!(context.contains("initial persistent peer harness"));
    assert!(context.contains("Cargo.toml"));
    assert!(context.contains("Target release: v0.1.0"));
    assert_eq!(
        config["defaults"]["model"].as_str(),
        Some("claude-sonnet-5")
    );
    assert_eq!(config["defaults"]["provider"].as_str(), Some("openai"));
    assert_eq!(
        config["defaults"]["base_url"].as_str(),
        Some("https://api.anthropic.com/v1")
    );
    for expected in [
        COMMITTED_FRAMEWORKS,
        "Codex owns login.",
        "Project preferences persist.",
        "Quiet ambient motion.",
    ] {
        assert!(context.contains(expected), "missing {expected}");
    }
    release::git(repo.path(), &["tag", "v0.1.0"]).await.unwrap();
    fs::write(
        repo.path().join("Cargo.toml"),
        "[workspace.package]\nversion=\"0.2.0\"\n",
    )
    .unwrap();
    let head = repo.commit("feat: journal relationships").await;
    let (text, previous) = notes::configuration(repo.path(), &head, version("0.2.0"))
        .await
        .unwrap();
    assert_eq!(previous.as_deref(), Some("v0.1.0"));
    assert!(!text.contains("Initial implementation inventory"));
}
#[tokio::test]
async fn actual_communique_compatible_adapter_reads_source_then_submits_notes() {
    let repo = Fixture::new().await;
    let api = Api::new(Reply::Notes(NOTES_BODY.into())).await;
    let output = repo.path().join("notes.md");
    repo.generate(&api, &output).await.unwrap();
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "# v0.1.0: Kuru 0.1.0\n\nPersistent peer conversations are now available."
    );
    let calls = api.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(call.path, "/v1/chat/completions");
        assert!(call.authorized);
        assert_eq!(call.body["model"], "claude-sonnet-5");
        assert_eq!(call.body["max_tokens"], 16384);
        for absent in [
            "thinking",
            "temperature",
            "reasoning_effort",
            "max_completion_tokens",
        ] {
            assert!(call.body.get(absent).is_none(), "unexpected {absent}");
        }
    }
    let request = &calls[0].body;
    assert!(
        request
            .to_string()
            .contains("initial persistent peer harness")
    );
    assert!(request.to_string().contains("Target release: v0.1.0"));
    assert!(request.get("thinking").is_none());
    let messages = calls[1].body["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| message["role"] == "assistant"
        && message["tool_calls"][0]["id"] == "read-fixture"));
    let result = messages
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(result["tool_call_id"], "read-fixture");
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains(COMMITTED_FRAMEWORKS)
    );
    assert_eq!(release::git(repo.path(), &["tag"]).await.unwrap(), "");
}
#[tokio::test]
async fn actual_api_errors_do_not_expose_payloads_or_leave_output_or_tags() {
    for reply in [Reply::Unauthorized, Reply::Malformed] {
        let repo = Fixture::new().await;
        let api = Api::new(reply).await;
        let output = repo.path().join("notes.md");
        let error = repo.generate(&api, &output).await.unwrap_err().to_string();
        assert!(error.contains("no release was published"));
        assert!(!error.contains("fixture-key"));
        assert!(!error.contains("provider payload"));
        assert!(!output.exists());
        assert_eq!(api.calls.lock().unwrap().len(), 1);
        assert_eq!(release::git(repo.path(), &["tag"]).await.unwrap(), "");
    }
}
#[tokio::test]
async fn notes_preserve_reviewed_output_and_reject_schema_or_version_mismatch() {
    let repo = Fixture::new().await;
    let head = repo.head().await;
    let output = repo.path().join("notes.md");
    fs::write(&output, "previous reviewed notes").unwrap();
    assert!(
        notes::generate(
            repo.path(),
            &head,
            version("0.1.0"),
            &output,
            &ProviderOptions::default()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("already exists")
    );
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "previous reviewed notes"
    );
    assert!(
        notes::configuration(repo.path(), &head, version("0.2.0"))
            .await
            .is_err()
    );
    fs::write(
        repo.path().join("communique.toml"),
        "[system]\nmodel=\"incorrect-schema\"\n",
    )
    .unwrap();
    let head = repo.commit("test: unsupported notes schema").await;
    assert!(
        notes::configuration(repo.path(), &head, version("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("configuration sections")
    );
    fs::write(
        repo.path().join("communique.toml"),
        "[defaults]\nmodel=[]\n",
    )
    .unwrap();
    let head = repo.commit("test: unsupported notes defaults").await;
    assert!(
        notes::configuration(repo.path(), &head, version("0.1.0"))
            .await
            .unwrap_err()
            .to_string()
            .contains("unsupported Communiqué default")
    );
}
#[tokio::test]
async fn real_release_cli_generates_notes_and_reports_invalid_sha() {
    let repo = Fixture::new().await;
    let api = Api::new(Reply::Notes(NOTES_BODY.into())).await;
    let output = repo.path().join("notes.md");
    let mut config: toml::Value = toml::from_str(include_str!("../../../communique.toml")).unwrap();
    config["defaults"].as_table_mut().unwrap().insert(
        "base_url".into(),
        toml::Value::String(format!("{}/v1", api.endpoint)),
    );
    fs::write(
        repo.path().join("communique.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let head = repo.commit("test: local notes endpoint").await;
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        tokio::process::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
            .arg("--root")
            .arg(repo.path())
            .args(["notes", "--version", "0.1.0", "--sha", &head, "--output"])
            .arg(&output)
            .env("OPENAI_API_KEY", "fixture-key")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("release CLI fixture timed out")
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        fs::read_to_string(output)
            .unwrap()
            .contains("Persistent peer conversations")
    );
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        tokio::process::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
            .arg("--root")
            .arg(repo.path())
            .args([
                "notes",
                "--version",
                "0.1.0",
                "--sha",
                "invalid",
                "--output",
                "unused",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("invalid-input CLI fixture timed out")
    .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("full lowercase commit SHA"));
}

#[tokio::test]
async fn snapshots_ignore_uncommitted_edits_and_generation_refuses_dirty_tools() {
    let repo = Fixture::new().await;
    let head = repo.head().await;
    fs::write(repo.path().join(FRAMEWORKS), "UNCOMMITTED FALSE DEFAULT").unwrap();
    fs::write(
        repo.path().join("communique.toml"),
        "context='UNCOMMITTED CONFIG'\n",
    )
    .unwrap();
    let (text, _) = notes::configuration(repo.path(), &head, version("0.1.0"))
        .await
        .unwrap();
    let config: toml::Value = toml::from_str(&text).unwrap();
    assert!(
        config["context"]
            .as_str()
            .unwrap()
            .contains(COMMITTED_FRAMEWORKS)
    );
    assert!(!text.contains("UNCOMMITTED"));
    let api = Api::new(Reply::Notes(NOTES_BODY.into())).await;
    let output = repo.path().join("notes.md");
    let error = repo.generate(&api, &output).await.unwrap_err().to_string();
    assert!(error.contains("uncommitted changes"), "{error}");
    assert!(api.calls.lock().unwrap().is_empty());
    assert!(!output.exists());
    repo.assert_no_tag().await;

    fs::write(
        repo.path().join("communique.toml"),
        include_str!("../../../communique.toml"),
    )
    .unwrap();
    fs::write(repo.path().join(FRAMEWORKS), "x".repeat(100_001)).unwrap();
    let head = repo.commit("docs: oversized framework context").await;
    let error = notes::configuration(repo.path(), &head, version("0.1.0"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("context exceeds limit"), "{error}");
}

#[tokio::test]
async fn actual_native_anthropic_adapter_still_rejects_claude_five_thinking() {
    let repo = Fixture::new().await;
    let api = Api::new(Reply::NativeThinking).await;
    let output = repo.path().join("notes.md");
    // Explicitly exercise the old adapter, with its own fake key. This must
    // reach the API and fail on the response shape, not missing credentials.
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        tokio::process::Command::new("communique")
            .arg("--config")
            .arg(repo.path().join("communique.toml"))
            .args([
                "generate",
                "v0.1.0",
                "--provider",
                "anthropic",
                "--model",
                "claude-sonnet-5",
                "--base-url",
                &api.endpoint,
                "--output",
            ])
            .arg(&output)
            .current_dir(repo.path())
            .kill_on_drop(true)
            .env("ANTHROPIC_API_KEY", "fixture-key")
            .env_remove("OPENAI_API_KEY")
            .env_remove("LLM_API_KEY")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .output(),
    )
    .await
    .expect("native protocol fixture timed out")
    .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("unknown variant `thinking`"), "{error}");
    println!(
        "Actual native adapter rejected the thinking response: {}",
        result.status
    );
    assert!(!output.exists());
    {
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, "/v1/messages");
        assert!(calls[0].authorized);
        assert_eq!(calls[0].body["model"], "claude-sonnet-5");
    }
    repo.assert_no_tag().await;
}

async fn assert_invalid_notes(body: String, expected: &str) {
    let repo = Fixture::new().await;
    let api = Api::new(Reply::Notes(body)).await;
    let output = repo.path().join("notes.md");
    let error = repo.generate(&api, &output).await.unwrap_err().to_string();
    assert!(error.contains(expected), "{error}");
    assert!(!output.exists());
    assert_eq!(api.calls.lock().unwrap().len(), 2);
    repo.assert_no_tag().await;
    fs::write(&output, "previous reviewed notes").unwrap();
    let error = repo.generate(&api, &output).await.unwrap_err().to_string();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "previous reviewed notes"
    );
    assert_eq!(api.calls.lock().unwrap().len(), 2);
}

async fn assert_retained_notes(body: String) {
    let repo = Fixture::new().await;
    let expected = format!("{NOTES_HEADING}{body}");
    let api = Api::new(Reply::Notes(body)).await;
    let output = repo.path().join("notes.md");
    repo.generate(&api, &output).await.unwrap();
    assert_eq!(fs::read(&output).unwrap(), expected.as_bytes());
    assert_eq!(api.calls.lock().unwrap().len(), 2);
    repo.assert_no_tag().await;

    // Retaining a readable draft must not introduce a second generation call
    // or replace it when the operation is repeated for editorial review.
    let error = repo.generate(&api, &output).await.unwrap_err().to_string();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(fs::read(&output).unwrap(), expected.as_bytes());
    assert_eq!(api.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn actual_communique_retains_seventeen_bullets_without_rewriting() {
    let body = (0..17).map(|i| format!("- Item {i}.\n")).collect();
    assert_retained_notes(body).await;
}

#[tokio::test]
async fn actual_communique_retains_451_words_without_rewriting() {
    let body = "word ".repeat(447);
    assert_eq!(
        format!("{NOTES_HEADING}{body}").split_whitespace().count(),
        451
    );
    assert_retained_notes(body).await;
}

#[tokio::test]
async fn current_source_guard_rejects_untracked_inputs_but_allows_ignored_builds() {
    let repo = Fixture::new().await;
    release::git(repo.path(), &["config", "status.showUntrackedFiles", "no"])
        .await
        .unwrap();
    let api = Api::new(Reply::Notes(NOTES_BODY.into())).await;
    let output = repo.path().join("notes.md");
    let stale = repo.path().join("stale-notes.md");
    fs::write(&stale, "Untracked stale behavior is not release source.").unwrap();
    let error = repo.generate(&api, &output).await.unwrap_err().to_string();
    assert!(error.contains("uncommitted changes"), "{error}");
    assert!(api.calls.lock().unwrap().is_empty());
    assert!(!output.exists());
    repo.assert_no_tag().await;
    fs::remove_file(stale).unwrap();
    fs::create_dir(repo.path().join("target")).unwrap();
    fs::write(
        repo.path().join("target/build-output"),
        "ignored build state",
    )
    .unwrap();
    repo.generate(&api, &output).await.unwrap();
    assert!(fs::read_to_string(output).unwrap().contains(NOTES_BODY));
}

#[tokio::test]
async fn actual_communique_retains_other_markdown_list_markers() {
    for marker in ["*", "+", "1.", "2)"] {
        assert_retained_notes(format!("  {marker} Item.\n").repeat(11)).await;
    }
}

#[tokio::test]
async fn actual_communique_retains_exact_byte_limit_but_rejects_oversized_output() {
    let body = "é".repeat((100_000 - NOTES_HEADING.len()) / 2);
    assert_eq!(NOTES_HEADING.len() + body.len(), 100_000);
    assert_retained_notes(body.clone()).await;
    assert_invalid_notes(format!("{body}x"), "notes must be a bounded regular file").await;
}
