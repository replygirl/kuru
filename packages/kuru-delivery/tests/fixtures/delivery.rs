//! Native delivery fixture. Invocations are explicit and never part of release artifacts.

use std::io::{self, Read, Write};

#[tokio::main]
async fn main() -> io::Result<()> {
    #[cfg(windows)]
    if std::fs::read(std::env::current_exe()?)?.ends_with(b"KURU_CANDIDATE_MARKER")
        && let Some(marker) = std::env::var_os("KURU_EXECUTION_MARKER")
    {
        std::fs::write(marker, b"candidate executed")?;
    }
    let mut arguments = std::env::args_os().skip(1);
    match arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .as_deref()
    {
        None | Some("--version") => println!("native fixture 0.2.0"),
        Some("echo") => {
            let values: Vec<_> = arguments.collect();
            println!(
                "{}",
                serde_json::to_string(&values).map_err(io::Error::other)?
            );
        }
        Some("copy") => {
            let mut bytes = Vec::new();
            io::stdin().take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 {
                return Err(io::Error::other("fixture input overflow"));
            }
            io::stdout().write_all(&bytes)?;
        }
        Some("mark") => {
            let path = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing marker path"))?;
            std::fs::write(path, b"candidate executed")?;
        }
        Some("check-hook-env") => {
            for (name, value) in [
                ("GIT_SSH_COMMAND", "fixture-ssh"),
                ("SSH_AUTH_SOCK", "fixture-agent"),
                ("GIT_ASKPASS", "fixture-askpass"),
                ("OPENAI_API_KEY", "fixture-inherited-key"),
            ] {
                if std::env::var(name).ok().as_deref() != Some(value) {
                    return Err(io::Error::other(
                        "fixture inherited environment was not preserved",
                    ));
                }
            }
        }
        #[cfg(windows)]
        Some("--internal-update-helper") => {
            let values: Vec<_> = arguments.collect();
            if let [encoded, flag, checkpoint] = values.as_slice()
                && flag == "--fixture-checkpoint"
            {
                kuru_delivery::update::test_support::run_helper_observed(
                    vec![encoded.clone()],
                    checkpoint
                        .to_str()
                        .ok_or_else(|| io::Error::other("invalid checkpoint"))?,
                )
                .await
                .map_err(io::Error::other)?;
            } else {
                kuru_delivery::update::run_helper(values)
                    .await
                    .map_err(io::Error::other)?;
            }
        }
        #[cfg(windows)]
        Some("crash-gap") => {
            let candidate = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing candidate"))?;
            let cache = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing helper cache"))?;
            kuru_delivery::update::test_support::hold_crash_gap(
                std::path::Path::new(&candidate),
                std::path::Path::new(&cache),
            )
            .await
            .map_err(io::Error::other)?;
        }
        #[cfg(windows)]
        Some(mode @ ("update" | "update-observed" | "update-parent-loss")) => {
            let candidate = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing candidate"))?;
            let cache = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing helper cache"))?;
            let checkpoint = if mode == "update-observed" {
                arguments
                    .next()
                    .ok_or_else(|| io::Error::other("missing checkpoint"))?
            } else {
                "none".into()
            };
            if arguments.next().is_some() {
                return Err(io::Error::other("unexpected update fixture arguments"));
            }
            let result = kuru_delivery::update::test_support::replace_running_binary_observed(
                std::path::Path::new(&candidate),
                std::path::Path::new(&cache),
                checkpoint
                    .to_str()
                    .ok_or_else(|| io::Error::other("invalid checkpoint"))?,
            )
            .await
            .map_err(io::Error::other)?;
            println!(
                "{}",
                serde_json::json!({"installed":result.installed,"cleanup_pending":result.cleanup_pending})
            );
            io::stdout().flush()?;
            if mode != "update" {
                println!(
                    "{}",
                    serde_json::json!({"parent_acknowledged":true,"parent_pid":std::process::id()})
                );
                io::stdout().flush()?;
                // The external test retains the enclosing Job. This skips
                // parent destructors while the actual helper observes its
                // inherited process handle and owns subsequent cleanup.
                std::process::exit(if mode == "update-parent-loss" { 42 } else { 0 });
            }
            // Parent-controlled lifetime: keep the old loaded image alive until
            // its owner has independently checked publication and cache bytes.
            let mut byte = [0];
            io::stdin().read_exact(&mut byte)?;
        }
        Some(other) => return Err(io::Error::other(format!("unknown fixture mode {other}"))),
    }
    Ok(())
}
