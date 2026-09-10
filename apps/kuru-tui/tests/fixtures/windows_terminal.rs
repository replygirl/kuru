#![forbid(unsafe_code)]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use kuru_platform::windows::{
        console::{ConsoleModeGuard, configure_test_baseline, inject_focus},
        process::{NativeSpawnSpec, StandardStream, inherited_stdio},
    };
    use serde::Deserialize;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        time::Duration,
    };

    fn publish(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
        let temporary = path.with_extension("pending");
        std::fs::write(&temporary, serde_json::to_vec(value)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
    #[derive(Deserialize)]
    struct Plan {
        mode: String,
        binary: PathBuf,
        args: Vec<String>,
        environment: BTreeMap<String, String>,
        cwd: PathBuf,
    }
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("fixture directory required")?,
    );
    let plan: Plan = serde_json::from_slice(&std::fs::read(directory.join("plan.json"))?)?;
    let before = configure_test_baseline()?;
    let observer = ConsoleModeGuard::capture()?;
    publish(
        &directory.join("before.json"),
        &json!({"input":before.input,"output":before.output}),
    )?;
    let result: anyhow::Result<i32> = async {
        if plan.mode == "partial-error" {
            struct Broken;
            impl std::io::Write for Broken {
                fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::other(
                        "injected terminal initialization write failure",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            let result = kuru::ui::TerminalSession::enter(&mut Broken);
            ensure!(
                result.is_err(),
                "partial initialization fixture did not fail"
            );
            return Ok(1);
        }
        if plan.mode == "error-unwind" {
            let result = (|| -> anyhow::Result<()> {
                let _session = kuru::ui::TerminalSession::enter(&mut std::io::stdout())?;
                anyhow::bail!("injected post-initialization terminal error")
            })();
            ensure!(result.is_err(), "error fixture did not fail");
            return Ok(1);
        }
        ensure!(plan.mode == "app", "unknown terminal fixture mode");
        let mut spec = NativeSpawnSpec::new(plan.binary, plan.cwd);
        spec.args = plan.args.into_iter().map(Into::into).collect();
        spec.environment = plan
            .environment
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        spec.stdin = inherited_stdio(StandardStream::Input)?;
        spec.stdout = inherited_stdio(StandardStream::Output)?;
        spec.stderr = inherited_stdio(StandardStream::Error)?;
        let mut child = spec.spawn().await?;
        let mut sequence = 0;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
        loop {
            if let Some(status) = child.try_wait()? {
                break Ok(status.code().unwrap_or(-1));
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "terminal fixture child deadline exceeded"
            );
            let command = directory.join(format!("control-{sequence}.json"));
            if command.exists() {
                let value: serde_json::Value = serde_json::from_slice(&std::fs::read(command)?)?;
                let focus = value["focus"].as_bool().context("focus command required")?;
                inject_focus(focus)?;
                publish(
                    &directory.join(format!("ack-{sequence}.json")),
                    &json!({"focus":focus}),
                )?;
                sequence += 1;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    .await;
    // Observation belongs to this sibling fixture, after the real application
    // has exited; it does not trust an application-generated restoration claim.
    let after = observer.current()?;
    publish(
        &directory.join("report.json"),
        &json!({
            "before":{"input":before.input,"output":before.output},
            "after":{"input":after.input,"output":after.output},
            "status":result.as_ref().ok(), "error":result.as_ref().err().map(|error|format!("{error:#}"))
        }),
    )?;
    result?;
    ensure!(before == after, "actual console modes were not restored");
    Ok(())
}
