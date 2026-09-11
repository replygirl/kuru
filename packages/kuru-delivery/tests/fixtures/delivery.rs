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
        #[cfg(windows)]
        Some("command-held-descendant") => {
            let path = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing lock path"))?;
            let lease = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)?;
            lease.try_lock().map_err(io::Error::other)?;
            io::stdout().write_all(b"R")?;
            io::stdout().flush()?;
            std::future::pending::<()>().await;
            drop(lease);
        }
        #[cfg(windows)]
        Some("command-output-before-tree-wait") => {
            use kuru_platform::windows::process::{Lifetime, NativeSpawnSpec, Stdio};
            use tokio::io::AsyncReadExt;
            let path = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing lock path"))?;
            let mut spec = NativeSpawnSpec::new(std::env::current_exe()?, std::env::current_dir()?);
            spec.args = vec!["command-held-descendant".into(), path];
            // Inherit the actual command owner's enclosing Job, but outlive this
            // fixture root. Dropping its local child handle must not stop it.
            spec.lifetime = Lifetime::TrustedSupervisor;
            spec.stdout = Stdio::Pipe;
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
            }
            let mut child = spec.spawn().await?;
            let mut ready = child
                .take_stdout()
                .ok_or_else(|| io::Error::other("missing readiness pipe"))?;
            let byte =
                tokio::time::timeout(std::time::Duration::from_secs(5), ready.read_u8()).await??;
            if byte != b'R' {
                return Err(io::Error::other("descendant did not retain its lock"));
            }
            ready.close(std::time::Duration::from_secs(5)).await?;
            io::stdout().write_all(b"native captured stdout prefix\n")?;
            io::stdout().write_all(&vec![b'x'; 70 * 1024])?;
            io::stdout().write_all(b"excluded stdout tail")?;
            io::stdout().flush()?;
            io::stderr().write_all(b"native fixture failure before tree wait\n")?;
            io::stderr().flush()?;
            return Err(io::Error::other("native root failed after both streams"));
        }
        #[cfg(windows)]
        Some("update-frame") => {
            let case = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(|| io::Error::other("missing update frame case"))?;
            if arguments.next().is_some() {
                return Err(io::Error::other("unexpected update frame argument"));
            }
            let (bytes, hold) = match case.as_str() {
                "absent" | "late-header" => (Vec::new(), true),
                "partial-header" => (vec![2], true),
                "partial-body" => (vec![2, 0, 0, 0, b'{'], true),
                "oversized" => (65537_u32.to_le_bytes().to_vec(), false),
                "truncated-header" => (vec![2, 0], false),
                "truncated-body" => (vec![2, 0, 0, 0, b'{'], false),
                "invalid-json" => (vec![1, 0, 0, 0, b'?'], false),
                "invalid-ack" => (vec![2, 0, 0, 0, b'{', b'}'], false),
                other => return Err(io::Error::other(format!("unknown frame case {other}"))),
            };
            io::stdout().write_all(&bytes)?;
            io::stdout().flush()?;
            io::stderr().write_all(b"ready\n")?;
            io::stderr().flush()?;
            if case == "late-header" {
                // Parent controls when the first byte arrives, after it has
                // started the bounded production receiver on the actual pipe.
                let mut byte = [0];
                io::stdin().read_exact(&mut byte)?;
                if byte != [b'r'] {
                    return Err(io::Error::other("invalid frame resume byte"));
                }
                io::stdout().write_all(&[2])?;
                io::stdout().flush()?;
            }
            if hold {
                let mut byte = [0];
                io::stdin().read_exact(&mut byte)?;
            }
        }
        #[cfg(windows)]
        Some("helper-stderr") => {
            let count: usize = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse().ok())
                .filter(|value| *value <= 256 * 1024)
                .ok_or_else(|| io::Error::other("invalid stderr fixture count"))?;
            io::stderr().write_all(b"native helper diagnostic prefix\n")?;
            for _ in 0..count.div_ceil(1024) {
                io::stderr().write_all(&[b'x'; 1024])?;
            }
            io::stdout().write_all(b"stderr fully written\n")?;
            io::stdout().flush()?;
            return Err(io::Error::other("native helper diagnostic final error"));
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
        Some(
            mode @ ("update" | "update-observed" | "update-observed-alive" | "update-parent-loss"),
        ) => {
            let candidate = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing candidate"))?;
            let cache = arguments
                .next()
                .ok_or_else(|| io::Error::other("missing helper cache"))?;
            let checkpoint = if matches!(mode, "update-observed" | "update-observed-alive") {
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
            if !matches!(mode, "update" | "update-observed-alive") {
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
