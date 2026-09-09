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
};
use tempfile::TempDir;
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
}
#[derive(Clone, Copy)]
enum Reply {
    Notes,
    Unauthorized,
    Thinking,
}
struct Api {
    endpoint: String,
    calls: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Api {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Api {
    async fn new(reply: Reply) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app=Router::new().fallback(move |State(calls):State<Arc<Mutex<Vec<Value>>>>, request:Request| async move {
            assert_eq!(request.uri().path(),"/v1/messages");
            assert_eq!(request.headers().get("x-api-key").unwrap(),"fixture-key");
            let request:Value=serde_json::from_slice(&to_bytes(request.into_body(),4*1024*1024).await.unwrap()).unwrap(); calls.lock().unwrap().push(request);
            let (status,value)=match reply {
                Reply::Notes=>(StatusCode::OK,json!({"content":[{"type":"tool_use","id":"submit-fixture","name":"submit_release_notes","input":{"release_title":"Kuru 0.1.0","release_body":"Persistent peer conversations are now available."}}],"stop_reason":"tool_use","usage":{"input_tokens":20,"output_tokens":10}})),
                Reply::Unauthorized=>(StatusCode::UNAUTHORIZED,json!({"error":{"type":"authentication_error","message":"fixture failure"}})),
                Reply::Thinking=>(StatusCode::OK,json!({"content":[{"type":"thinking","thinking":"","signature":"fixture"},{"type":"text","text":"# Notes\nUnsupported response shape."}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":10}})),
            }; (status,axum::Json(value)).into_response()
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
            endpoint: Some(self.endpoint.clone()),
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
        Some("claude-haiku-4-5-20251001")
    );
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
async fn actual_communique_uses_configured_model_and_atomically_writes_factual_notes() {
    let repo = Fixture::new().await;
    let api = Api::new(Reply::Notes).await;
    let output = repo.path().join("notes.md");
    notes::generate(
        repo.path(),
        &repo.head().await,
        version("0.1.0"),
        &output,
        &api.options(),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        "# v0.1.0: Kuru 0.1.0\n\nPersistent peer conversations are now available."
    );
    let calls = api.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let request = &calls[0];
    assert_eq!(request["model"], "claude-haiku-4-5-20251001");
    assert!(
        request
            .to_string()
            .contains("initial persistent peer harness")
    );
    assert!(request.to_string().contains("Target release: v0.1.0"));
    assert!(request.get("thinking").is_none());
    assert_eq!(release::git(repo.path(), &["tag"]).await.unwrap(), "");
}
#[tokio::test]
async fn actual_api_failure_and_unsupported_thinking_do_not_leave_output_or_tags() {
    for reply in [Reply::Unauthorized, Reply::Thinking] {
        let repo = Fixture::new().await;
        let api = Api::new(reply).await;
        let output = repo.path().join("notes.md");
        let error = notes::generate(
            repo.path(),
            &repo.head().await,
            version("0.1.0"),
            &output,
            &api.options(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("no release was published"));
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
    let api = Api::new(Reply::Notes).await;
    let output = repo.path().join("notes.md");
    let head = repo.head().await;
    let mut config: toml::Value = toml::from_str(include_str!("../../../communique.toml")).unwrap();
    config["defaults"]
        .as_table_mut()
        .unwrap()
        .insert("base_url".into(), toml::Value::String(api.endpoint.clone()));
    fs::write(
        repo.path().join("communique.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
        .arg("--root")
        .arg(repo.path())
        .args(["notes", "--version", "0.1.0", "--sha", &head, "--output"])
        .arg(&output)
        .env("ANTHROPIC_API_KEY", "fixture-key")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .output()
        .await
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
    let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_kuru-release"))
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
        .output()
        .await
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("full lowercase commit SHA"));
}
