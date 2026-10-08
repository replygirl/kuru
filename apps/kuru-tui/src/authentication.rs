//! Presentation for Kuru-owned OpenAI authentication. The connector owns tokens
//! and HTTP; this module never opens credential files or receives token values.
#[cfg(unix)]
use std::time::Duration;
use std::{future::Future, io::Write, path::Path};

use crate::cli::Command;
#[cfg(unix)]
use anyhow::ensure;
use anyhow::{Context, Result, bail};
use kuru_connectors::{
    AuthManager, AuthStatus, CanaryReport, discover_subscription_models, subscription_canary,
};
use kuru_core::ModelInfo;

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
            complete_login(
                login.finish(),
                discover_subscription_models(&auth),
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            )
            .await?;
        }
        Command::Login { no_browser, .. } => {
            let login = auth.begin_browser().await?;
            println!("Sign in to ChatGPT:\n{}", login.authorization_url());
            if !no_browser && open_browser(login.authorization_url()).await.is_err() {
                eprintln!("Open the URL above in your browser to continue.");
            }
            complete_login(
                login.finish(),
                discover_subscription_models(&auth),
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            )
            .await?;
        }
        _ => unreachable!("only authentication commands are routed here"),
    }
    Ok(())
}

async fn complete_login(
    login: impl Future<Output = Result<AuthStatus>>,
    catalog: impl Future<Output = Result<Vec<ModelInfo>>>,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> Result<()> {
    finish(login).await?;
    writeln!(output, "Signed in to ChatGPT.")?;
    report_login_catalog(catalog.await, output, errors)?;
    Ok(())
}

fn report_login_catalog(
    catalog: Result<Vec<ModelInfo>>,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> std::io::Result<()> {
    match catalog {
        Ok(models) => {
            writeln!(output, "Available ChatGPT models:")?;
            for model in models {
                writeln!(output, "  {}", model.id.escape_debug())?;
            }
            Ok(())
        }
        Err(_) => writeln!(
            errors,
            "Signed-in model listing is unavailable. Retry with kuru --provider codex models."
        ),
    }
}

pub(crate) async fn canary(model: &str, data: &Path, cwd: &Path) -> CanaryReport {
    #[cfg(feature = "test-support")]
    if let Some(endpoint) = std::env::var_os("KURU_TEST_CHATGPT_BASE") {
        let Some(endpoint) = endpoint.to_str() else {
            return CanaryReport {
                schema_version: 1,
                state: kuru_connectors::CanaryState::Unverified,
                reason: Some("test_endpoint_rejected"),
                incompatibility: None,
                action: Some("Use a numeric loopback endpoint in the test fixture."),
                observed: Vec::new(),
                unobserved: vec![
                    kuru_connectors::CanaryStage::Credentials,
                    kuru_connectors::CanaryStage::Catalog,
                    kuru_connectors::CanaryStage::Completion,
                    kuru_connectors::CanaryStage::Usage,
                    kuru_connectors::CanaryStage::Refresh,
                    kuru_connectors::CanaryStage::ToolCall,
                    kuru_connectors::CanaryStage::Reasoning,
                ],
            };
        };
        return kuru_connectors::subscription_canary_test_endpoint(data, cwd, model, endpoint)
            .await;
    }
    subscription_canary(data, cwd, model).await
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
        kuru_memory::test_support::closing(async {
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
        })
        .await
    }
}

#[cfg(windows)]
pub(crate) async fn open_browser(url: &str) -> Result<()> {
    kuru_platform::windows::browser::open_http_url(url)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    fn authenticated() -> AuthStatus {
        AuthStatus {
            authenticated: true,
            account_id: None,
            expires_at: None,
            api_key_available: false,
        }
    }

    #[tokio::test]
    async fn successful_login_reports_success_before_polling_catalog_and_lists_models() {
        use std::{cell::RefCell, rc::Rc};

        struct CapturedOutput(Rc<RefCell<Vec<u8>>>);
        impl Write for CapturedOutput {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured = Rc::new(RefCell::new(Vec::new()));
        let mut output = CapturedOutput(captured.clone());
        let mut errors = Vec::new();
        complete_login(
            async { Ok(authenticated()) },
            async {
                assert_eq!(captured.borrow().as_slice(), b"Signed in to ChatGPT.\n");
                Ok(vec![ModelInfo {
                    id: "future-model".into(),
                    name: "Future model".into(),
                    efforts: vec![],
                    default_effort: None,
                    metadata: Default::default(),
                }])
            },
            &mut output,
            &mut errors,
        )
        .await
        .unwrap();
        assert_eq!(
            captured.borrow().as_slice(),
            b"Signed in to ChatGPT.\nAvailable ChatGPT models:\n  future-model\n"
        );
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn failed_catalog_keeps_successful_login_and_reports_safe_retry_guidance() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        complete_login(
            async { Ok(authenticated()) },
            async { Err(anyhow::anyhow!("provider-body-secret-sentinel")) },
            &mut output,
            &mut errors,
        )
        .await
        .unwrap();
        assert_eq!(output, b"Signed in to ChatGPT.\n");
        assert_eq!(
            errors,
            b"Signed-in model listing is unavailable. Retry with kuru --provider codex models.\n"
        );
    }

    #[tokio::test]
    async fn failed_login_never_polls_catalog_or_reports_success() {
        let polled = std::cell::Cell::new(false);
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let result = complete_login(
            async { Err(anyhow::anyhow!("synthetic login rejected")) },
            async {
                polled.set(true);
                Ok(vec![])
            },
            &mut output,
            &mut errors,
        )
        .await;
        assert!(result.is_err());
        assert!(!polled.get());
        assert!(output.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn login_catalog_shows_only_safe_model_ids_and_fixed_failure_guidance() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        report_login_catalog(
            Ok(vec![ModelInfo {
                id: "future-model\n\u{1b}[31m".into(),
                name: "provider-name-secret-sentinel".into(),
                efforts: vec!["provider-effort-secret-sentinel".into()],
                default_effort: None,
                metadata: Default::default(),
            }]),
            &mut output,
            &mut errors,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Available ChatGPT models:\n  future-model\\n\\u{1b}[31m\n"
        );
        assert!(errors.is_empty());

        let mut output = Vec::new();
        report_login_catalog(
            Err(anyhow::anyhow!("provider-body-secret-sentinel")),
            &mut output,
            &mut errors,
        )
        .unwrap();
        assert!(output.is_empty());
        assert_eq!(
            String::from_utf8(errors).unwrap(),
            "Signed-in model listing is unavailable. Retry with kuru --provider codex models.\n"
        );
    }
}
