//! Presentation for Kuru-owned OpenAI authentication. The connector owns tokens
//! and HTTP; this module never opens credential files or receives token values.
#[cfg(unix)]
use std::time::Duration;
use std::{future::Future, path::Path};

use crate::cli::Command;
#[cfg(unix)]
use anyhow::ensure;
use anyhow::{Context, Result, bail};
use kuru_connectors::{AuthManager, AuthStatus};

pub(crate) async fn run(
    command: &Command,
    responses_api_key_env: Option<&str>,
    data: &Path,
    cwd: &Path,
) -> Result<()> {
    let api_key = if let Some(name) = responses_api_key_env {
        match std::env::var(name) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => {
                bail!("configured Responses API-key environment variable must contain valid text")
            }
        }
    } else {
        None
    };
    let auth = AuthManager::new(data.to_owned(), cwd.to_owned(), api_key)?;
    match command {
        Command::Auth => println!("{}", serde_json::to_string_pretty(&auth.status().await?)?),
        Command::Logout => {
            auth.logout().await?;
            println!("Signed out of ChatGPT in Kuru. Environment API keys are unchanged.");
        }
        Command::Login { device: true, .. } => {
            let login = auth.begin_device().await?;
            println!(
                "Open {} and enter code {}",
                login.verification_url(),
                login.user_code()
            );
            finish(login.finish()).await?;
            println!("Signed in to ChatGPT.");
        }
        Command::Login { no_browser, .. } => {
            let login = auth.begin_browser().await?;
            println!("Sign in to ChatGPT:\n{}", login.authorization_url());
            if !no_browser && open_browser(login.authorization_url()).await.is_err() {
                eprintln!("Open the URL above in your browser to continue.");
            }
            finish(login.finish()).await?;
            println!("Signed in to ChatGPT.");
        }
        _ => unreachable!("only authentication commands are routed here"),
    }
    Ok(())
}

async fn finish(login: impl Future<Output = Result<AuthStatus>>) -> Result<AuthStatus> {
    tokio::select! {
        result = login => result,
        signal = tokio::signal::ctrl_c() => {
            signal.context("listen for login cancellation")?;
            bail!("login cancelled");
        }
    }
}

#[cfg(unix)]
pub(crate) async fn open_browser(url: &str) -> Result<()> {
    use std::process::Stdio;
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let child = tokio::process::Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    browser_handoff(child, Duration::from_secs(3)).await
}

#[cfg(unix)]
async fn browser_handoff(mut child: tokio::process::Child, deadline: Duration) -> Result<()> {
    // Desktop launchers may wait for the browser session itself. Reap the
    // launcher when it exits, but never terminate the user's browser lifetime.
    let completion = tokio::spawn(async move { child.wait().await });
    if let Ok(result) = tokio::time::timeout(deadline, completion).await {
        ensure!(result??.success(), "browser opener failed");
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desktop_handoff_reports_failure_and_keeps_a_running_browser_alive() {
        // Held across each spawn; see `crate::spawn_gate`.
        let failed = {
            let _gate = crate::spawn_gate::spawning().await;
            tokio::process::Command::new("/bin/sh")
                .args(["-c", "exit 7"])
                .spawn()
                .unwrap()
        };
        assert!(
            browser_handoff(failed, Duration::from_secs(2))
                .await
                .is_err()
        );

        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("browser-alive");
        let running = {
            let _gate = crate::spawn_gate::spawning().await;
            tokio::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "sleep 0.1; printf desktop > \"$1\"",
                    "browser-fixture",
                ])
                .arg(&marker)
                .spawn()
                .unwrap()
        };
        browser_handoff(running, Duration::from_millis(5))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while std::fs::read_to_string(&marker).ok().as_deref() != Some("desktop") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "desktop");
    }
}

#[cfg(windows)]
pub(crate) async fn open_browser(url: &str) -> Result<()> {
    kuru_platform::windows::browser::open_http_url(url)
        .await
        .map_err(Into::into)
}
