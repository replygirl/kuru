#[cfg(windows)]
use kuru_platform::windows::process::{Console, NativeChild as Child, NativeSpawnSpec, Stdio};
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

#[path = "support/memory.rs"]
mod memory;

struct Server(Child);

const MAX_STARTUP_LINES: usize = 16;

fn expected_startup_line(line: &str) -> bool {
    matches!(
        line.trim_end(),
        "Memory: waiting for project ownership…"
            | "Memory: waiting for verified runtime cache…"
            | "Memory: verifying cached runtime…"
            | "Memory: checking runtime version…"
            | "Memory: preparing database…"
            | "Memory: opening database…"
            | "Memory: ready."
    ) || line.starts_with("Memory is ready at ")
}

impl Drop for Server {
    fn drop(&mut self) {
        #[cfg(unix)]
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
        #[cfg(windows)]
        let _ = self.0.terminate();
    }
}

#[tokio::test]
async fn authenticated_a2a_cli_routes_a_part_and_shuts_down_cleanly() -> Result<()> {
    let root = tempfile::tempdir()?;
    let project = root.path().join("workspace");
    let data = root.path().join("data");
    std::fs::create_dir(&project)?;
    #[cfg(unix)]
    let (mut child, line) = {
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
            .env("XDG_CONFIG_HOME", memory::configuration(root.path())?)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut child = Server(child);
        let stderr = child.0.stderr.take().context("server stderr missing")?;
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let result = (|| -> Result<String> {
                let mut reader = BufReader::new(stderr);
                for _ in 0..MAX_STARTUP_LINES {
                    let mut line = String::new();
                    reader.read_line(&mut line)?;
                    if line.starts_with("Kuru A2A listening on ") {
                        return Ok(line);
                    }
                    ensure!(
                        expected_startup_line(&line),
                        "unexpected server startup frame: {line:?}"
                    );
                }
                anyhow::bail!(
                    "server did not publish a listening line within {MAX_STARTUP_LINES} startup frames"
                );
            })();
            let _ = sender.send(result);
        });
        let line = receiver.recv_timeout(Duration::from_secs(10))??;
        reader.join().expect("server readiness reader panicked");
        (child, line)
    };
    #[cfg(windows)]
    let (mut child, line) = {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut spec =
            NativeSpawnSpec::new(env!("CARGO_BIN_EXE_kuru").into(), root.path().to_path_buf());
        spec.args = vec![
            "-C".into(),
            project.as_os_str().into(),
            "--data-dir".into(),
            data.as_os_str().into(),
        ];
        spec.args.extend(
            [
                "--provider",
                "demo",
                "--mode",
                "freudian",
                "--no-dream",
                "serve",
                "--bind",
                "127.0.0.1:0",
            ]
            .map(Into::into),
        );
        spec.environment = std::env::vars_os()
            .filter(|(key, _)| {
                !kuru_platform::windows::process::environment_key_eq(
                    key,
                    std::ffi::OsStr::new("XDG_CONFIG_HOME"),
                ) && !kuru_platform::windows::process::environment_key_eq(
                    key,
                    std::ffi::OsStr::new("KURU_A2A_TOKEN"),
                )
            })
            .collect();
        spec.environment.extend([
            (
                "XDG_CONFIG_HOME".into(),
                memory::configuration(root.path())?.into_os_string(),
            ),
            ("KURU_A2A_TOKEN".into(), "integration-token-123456".into()),
        ]);
        spec.console = Console::NewProcessGroup;
        spec.stderr = Stdio::Pipe;
        let mut child = Server(spec.spawn().await?);
        let stderr = child.0.take_stderr().context("server stderr missing")?;
        let mut reader = BufReader::new(stderr);
        let line = tokio::time::timeout(Duration::from_secs(80), async {
            for _ in 0..MAX_STARTUP_LINES {
                let mut line = String::new();
                reader.read_line(&mut line).await?;
                if line.starts_with("Kuru A2A listening on ") {
                    return Ok::<_, std::io::Error>(line);
                }
                if !expected_startup_line(&line) {
                    return Err(std::io::Error::other(format!(
                        "unexpected server startup frame: {line:?}"
                    )));
                }
            }
            Err(std::io::Error::other(format!(
                "server did not publish a listening line within {MAX_STARTUP_LINES} startup frames"
            )))
        })
        .await??;
        // The server only publishes one readiness line. Retain no unobserved
        // pipe operation after completing that frame.
        reader.into_inner().close(Duration::from_secs(5)).await?;
        (child, line)
    };
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
    #[cfg(windows)]
    child.0.interrupt()?;
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
