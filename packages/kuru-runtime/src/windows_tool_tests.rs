//! Native authority replay through the real actor -> ToolHost -> tool receipt
//! path. Only model completions are simulated; files, PowerShell and Dolt are real.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use kuru_connectors::Provider;
use kuru_core::{Completion, CompletionRequest, Config, Mode, ModelInfo, ToolCall};
use kuru_memory::MemoryStore;
use serde_json::{Value, json};

use crate::Harness;

#[derive(Default)]
struct Observed {
    next: usize,
    receipts: BTreeMap<String, String>,
    recipients: BTreeSet<String>,
}

struct Replay {
    calls: Vec<ToolCall>,
    observed: Mutex<Observed>,
}

#[async_trait]
impl Provider for Replay {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let mut reply = Completion {
            text: "No additional peer work needed".into(),
            calls: vec![],
            input_tokens: 7,
            output_tokens: 3,
        };
        if !request.instructions.contains("Phase: speak") {
            return Ok(reply);
        }
        let mut observed = self.observed.lock().unwrap();
        for message in request
            .messages
            .iter()
            .filter(|message| message.role == "tool")
        {
            let receipt: Value = serde_json::from_str(&message.content)?;
            let id = receipt["call_id"]
                .as_str()
                .context("tool receipt call ID")?;
            let output = receipt["output"].as_str().context("tool receipt output")?;
            observed.receipts.insert(id.into(), output.into());
            observed.recipients.insert(request.actor.clone());
        }
        if let Some(call) = self.calls.get(observed.next) {
            ensure!(
                request.tools.iter().any(|tool| tool.name == call.name),
                "authorized tool is absent from the model's catalog: {}",
                call.name
            );
            reply.text = "Running the next native tool request".into();
            reply.calls.push(call.clone());
            observed.next += 1;
        } else {
            reply.text = "Native tool replay complete".into();
        }
        Ok(reply)
    }
}

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn native_model_tool_replay_preserves_authority_and_returns_real_receipts() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project 日本語");
    std::fs::create_dir(&project)?;
    let outside = temporary.path().join("outside.txt");
    std::fs::write(&outside, b"outside bytes must survive")?;
    std::fs::hard_link(&outside, project.join("alias"))?;
    std::fs::write(project.join(".env"), b"private fixture value")?;
    std::fs::write(project.join("ordinary"), b"ordinary bytes")?;
    let token = "sk-proj-abcdefghijklmnop0123456789";
    let marker = "[REDACTED:recognized-secret]";
    let projection_control = "projection-control-remains-exact";
    let projection_source = format!("openai_api_key={token}\n{projection_control}");
    let projected_file_output = format!("openai_api_key={marker}\n{projection_control}");
    let projection_path = project.join("projection.txt");
    std::fs::write(&projection_path, &projection_source)?;
    let literal_path = "literal & name.txt";
    let literal = "$(Set-Content escaped.txt changed) & | %PATH% ! 日本語";
    let shell_control = "literal & | < > ^ %PATH% ! 日本語";
    let shell_stdout = format!("{shell_control}; openai_api_key={token}");
    let projected_shell_stdout = format!("{shell_control}; openai_api_key={marker}");
    let mut calls = vec![
        call(
            "create",
            "file_write",
            json!({"path":literal_path,"content":literal}),
        ),
        call("read", "file_read", json!({"path":literal_path})),
        call(
            "redacted-read",
            "file_read",
            json!({"path":"projection.txt"}),
        ),
        call("list", "file_list", json!({"path":"."})),
        call(
            "shell",
            "shell",
            json!({
                "command":format!("[Console]::Out.Write('{shell_stdout}'); [Console]::Error.Write('native stderr'); [IO.File]::WriteAllText([IO.Path]::Combine((Get-Location).Path,'shell-created.txt'),'shell bytes'); exit 7")
            }),
        ),
        call(
            "shell-read",
            "file_read",
            json!({"path":"shell-created.txt"}),
        ),
        call("delete", "file_delete", json!({"path":literal_path})),
        call(
            "shell-delete",
            "file_delete",
            json!({"path":"shell-created.txt"}),
        ),
    ];
    for (id, path) in [
        ("traversal", "..\\outside.txt"),
        ("trailing", ".env "),
        ("stream", "ordinary:stream"),
        ("device", "NUL"),
        ("hardlink", "alias"),
        ("metachar", "literal | Set-Content escaped.txt"),
    ] {
        calls.push(call(
            id,
            "file_write",
            json!({"path":path,"content":"unauthorized change"}),
        ));
    }
    calls.push(call("private-read", "file_read", json!({"path":".env."})));
    calls.push(call("hardlink-read", "file_read", json!({"path":"alias"})));
    calls.push(call(
        "hardlink-delete",
        "file_delete",
        json!({"path":"alias"}),
    ));
    let replay = Arc::new(Replay {
        calls,
        observed: Mutex::new(Observed::default()),
    });
    let config = Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "native-replay".into(),
        max_rounds: 1,
        max_tool_calls: replay.calls.len() + 1,
        allow_write: true,
        allow_shell: true,
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    };
    let mut harness = Harness::new(
        config,
        &project,
        MemoryStore::temporary().await?,
        replay.clone(),
        None,
    )
    .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        harness.run("Exercise the scripted native file and shell requests"),
    )
    .await
    .context("native model tool replay exceeded its deadline");
    let closed = harness.shutdown(false).await;
    let output = result??;
    closed?;
    ensure!(
        output.text == "Native tool replay complete",
        "replay did not finish: {output:?}"
    );
    ensure!(
        output
            .events
            .iter()
            .filter(|event| event.kind == "tool")
            .count()
            == replay.calls.len(),
        "not every model request reached the actual tool dispatcher"
    );
    let observed = replay.observed.lock().unwrap();
    ensure!(
        observed.next == replay.calls.len(),
        "script did not issue all requests"
    );
    ensure!(
        observed.receipts.len() == replay.calls.len(),
        "model did not receive every receipt"
    );
    ensure!(
        observed.recipients.len() == 1,
        "private receipts reached another speaking identity"
    );
    for call in &replay.calls {
        ensure!(
            observed.receipts.contains_key(&call.id),
            "missing receipt for {}",
            call.id
        );
    }
    let receipts = &observed.receipts;
    ensure!(
        receipts["create"].starts_with("Wrote "),
        "{}",
        receipts["create"]
    );
    ensure!(
        receipts["read"] == literal,
        "file argument data was interpreted"
    );
    ensure!(
        receipts["redacted-read"] == projected_file_output,
        "file result was not projected: {}",
        receipts["redacted-read"]
    );
    let listed: Value = serde_json::from_str(&receipts["list"])
        .with_context(|| format!("file-list receipt was not JSON: {}", receipts["list"]))?;
    ensure!(
        listed[literal_path] == "file" && listed.get(".env").is_none(),
        "unexpected listing: {listed}"
    );
    let shell: Value = serde_json::from_str(&receipts["shell"])
        .with_context(|| format!("shell receipt was not JSON: {}", receipts["shell"]))?;
    ensure!(
        shell["exit_code"] == 7 && shell["success"] == false,
        "wrong native status: {shell}"
    );
    let stdout = shell["stdout"].as_str().context("native stdout")?;
    ensure!(
        stdout == projected_shell_stdout,
        "wrong projected native stdout: {shell}"
    );
    ensure!(
        shell["stderr"] == "native stderr",
        "wrong native output: {shell}"
    );
    ensure!(
        receipts["shell-read"] == "shell bytes",
        "shell did not perform its authorized write"
    );
    for id in ["delete", "shell-delete"] {
        ensure!(receipts[id] == "Deleted file", "{id}: {}", receipts[id]);
    }
    for id in [
        "traversal",
        "trailing",
        "stream",
        "device",
        "hardlink",
        "metachar",
        "private-read",
        "hardlink-read",
        "hardlink-delete",
    ] {
        ensure!(
            receipts[id].starts_with("ERROR: "),
            "{id} did not reach the model as an error: {}",
            receipts[id]
        );
    }
    ensure!(
        std::fs::read(&outside)? == b"outside bytes must survive",
        "outside file changed"
    );
    ensure!(
        std::fs::read(project.join("alias"))? == b"outside bytes must survive",
        "hardlink was changed or removed"
    );
    ensure!(
        std::fs::read(project.join(".env"))? == b"private fixture value",
        "protected file changed"
    );
    ensure!(
        std::fs::read(project.join("ordinary"))? == b"ordinary bytes",
        "stream base changed"
    );
    ensure!(
        std::fs::read_to_string(&projection_path)? == projection_source,
        "projected file source was changed"
    );
    ensure!(
        !project.join("ordinary:stream").exists(),
        "alternate stream was created"
    );
    for absent in [literal_path, "shell-created.txt", "escaped.txt"] {
        ensure!(
            !project.join(absent).exists(),
            "unexpected side effect at {absent}"
        );
    }
    Ok(())
}
