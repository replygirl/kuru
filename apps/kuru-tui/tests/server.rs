use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[tokio::test]
async fn authenticated_a2a_cli_routes_a_part_and_shuts_down_cleanly() -> Result<()> {
    let root = tempfile::tempdir()?;
    let project = root.path().join("workspace");
    let data = root.path().join("data");
    std::fs::create_dir(&project)?;
    let child = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args([
            "--provider",
            "demo",
            "--mode",
            "freudian",
            "--no-dream",
            "serve",
            "--bind",
            "127.0.0.1:0",
        ])
        .env("KURU_A2A_TOKEN", "integration-token-123456")
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut child = Server(child);
    let stderr = child.0.stderr.take().context("server stderr missing")?;
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stderr).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver.recv_timeout(Duration::from_secs(10))??;
    reader.join().expect("server readiness reader panicked");
    ensure!(line.contains("listening"), "{line}");
    let address = line
        .split_whitespace()
        .last()
        .context("missing server address")?;
    let base = format!("http://{address}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let card: Value = client
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(card["supportedInterfaces"][0]["url"], base);
    let payload = json!({
        "jsonrpc":"2.0", "id":1, "method":"SendMessage", "params":{
            "message":{"messageId":"external-test", "role":"ROLE_USER",
                "parts":[{"text":"A2A e2e message"}]}
        }
    });
    let response = client
        .post(format!("{base}/"))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response: Value = client
        .post(format!("{base}/agents/ego"))
        .json(&payload)
        .bearer_auth("integration-token-123456")
        .header("A2A-Version", "1.0")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(response["result"]["message"]["role"], "ROLE_AGENT");
    assert!(
        response["result"]["message"]["parts"][0]["text"]
            .as_str()
            .context("agent response text missing")?
            .contains("demo")
    );
    let card: Value = client
        .get(format!("{base}/agents/ego/.well-known/agent-card.json"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        card["supportedInterfaces"][0]["url"]
            .as_str()
            .context("part interface URL missing")?
            .starts_with(&format!("{base}/agents/"))
    );
    #[cfg(unix)]
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.0.id().try_into()?),
        nix::sys::signal::Signal::SIGINT,
    )?;
    #[cfg(not(unix))]
    child.0.kill()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "server failed graceful shutdown: {status}"
            );
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "server failed to stop after interrupt"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
