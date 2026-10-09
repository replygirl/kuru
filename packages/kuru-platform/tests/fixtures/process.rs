//! Native fixture executable; never a product entrypoint.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(guard()?, run()).await?
}

/// This fixture's self-timeout, which only reaps an orphan and never decides
/// an outcome. A parked mode is ended by its parent's termination, but a
/// `Lifetime::TrustedSupervisor` handle does not terminate on drop, so a
/// parent that panics first would leave it parked for good.
///
/// A parent that parks this fixture, or whose own step ends it, passes the
/// self-timeout in milliseconds as the argument after the mode's fixed
/// arguments, and this process uses it as is. The parent derives it at
/// `budget_arg` in `tests/windows_process.rs`: a series of the parent's waits
/// from the spawn until it no longer needs this process running, plus one
/// `LIMIT` for the parent's unbounded steps after its last counted wait and
/// the gap between the two clocks.
///
/// A launch that passes no budget keeps the existing 30 s backstop, not
/// derived here: the parked `capture-stall` and `capture-flood` modes, whose
/// parent is `tests/windows_commands.rs`; the `console-owner` mode's `idle`
/// child, ended by `interrupt()`; `startup`, which polls for a release file
/// its test never writes, so only the close of the `owner-startup` fixture's
/// `OwnedJob` at that owner's termination ends it; `rendezvous`, ended when
/// its parent (the test, or `trusted-owner` at its termination) drops or
/// closes the pipe; `duplex`, ended by its parent's payload; and the remaining
/// modes, which end on their own work or on input their parent supplies.
#[cfg(windows)]
fn guard() -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let budget = match args.first().and_then(|mode| mode.to_str()) {
        Some("idle") => args.get(1),
        Some("rendezvous-stall" | "rendezvous-partial-frame" | "trusted-owner") => args.get(2),
        Some("owner" | "owner-startup") => args.get(4),
        _ => None,
    };
    Ok(match budget {
        Some(budget) => std::time::Duration::from_millis(
            budget.to_str().ok_or("non-UTF-8 fixture budget")?.parse()?,
        ),
        None => std::time::Duration::from_secs(30),
    })
}

#[cfg(windows)]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    use kuru_platform::windows::{
        pipe,
        process::{Console, Lifetime, NativeSpawnSpec, Stdio},
    };
    use std::{
        fs::{self, File},
        io::{Read, Seek, SeekFrom, Write},
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
        process::Command,
        time::{Duration, Instant},
    };
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let child_spec = || -> std::io::Result<NativeSpawnSpec> {
        let mut spec = NativeSpawnSpec::new(std::env::current_exe()?, std::env::current_dir()?);
        for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(key) {
                spec.environment.push((key.into(), value));
            }
        }
        Ok(spec)
    };
    let mode = args
        .first()
        .and_then(|s| s.to_str())
        .ok_or("missing fixture mode")?;
    match mode {
        "current-image" => {
            eprintln!("current-image: acquiring initial guard");
            let (mut image, diagnostic_file, initial_error) =
                match kuru_platform::windows::process::current_image() {
                    Ok(image) => (Some(image), None, None),
                    Err(error) => {
                        let diagnostic = serde_json::json!({
                            "stage":"initial_guard", "error":error.to_string(),
                            "kind":format!("{:?}", error.kind()), "os_error":error.raw_os_error(),
                        });
                        eprintln!("current-image: diagnostic-only fallback after {diagnostic}");
                        // This is exclusively a fixture handle for completing the
                        // rename experiment. It confers no current-image trust, and
                        // the parent MUST fail after collecting both observations.
                        let file = File::options()
                        .read(true)
                        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
                        .open(std::env::current_exe()?)?;
                        (None, Some(file), Some(diagnostic))
                    }
                };
            if let Some(image) = image.as_mut() {
                let invocation = std::env::current_exe()?;
                let checked_path = image.path().to_path_buf();
                let file = image.file_mut();
                file.seek(SeekFrom::Start(0))?;
                let mut before = [0; 2];
                file.read_exact(&mut before)?;
                file.seek(SeekFrom::Start(0))?;
                let refusal = file.write(b"XX");
                file.seek(SeekFrom::Start(0))?;
                let mut after = [0; 2];
                file.read_exact(&mut after)?;
                assert_eq!(checked_path, invocation, "checked current-image pathname");
                assert_eq!(&before, b"MZ", "read the actual Windows executable header");
                assert_eq!(
                    refusal.unwrap_err().kind(),
                    std::io::ErrorKind::PermissionDenied
                );
                assert_eq!(
                    after, before,
                    "mutable reader granted no image write authority"
                );
            }
            eprintln!("current-image: reading held identity");
            let held_file = image
                .as_ref()
                .map(|image| image.file())
                .or(diagnostic_file.as_ref())
                .ok_or("missing image experiment handle")?;
            let identity = kuru_platform::fs::regular_file_info(held_file)?
                .identity
                .to_bytes();
            println!(
                "{}",
                serde_json::json!({
                    "ready":"held", "identity":identity,
                    "diagnostic_only":diagnostic_file.is_some(), "initial_error":initial_error,
                })
            );
            std::io::stdout().flush()?;
            let mut byte = [0];
            std::io::stdin().read_exact(&mut byte)?;
            drop(image);
            drop(diagnostic_file);
            eprintln!("current-image: initial guard released");
            println!("released");
            std::io::stdout().flush()?;
            std::io::stdin().read_exact(&mut byte)?;
            eprintln!("current-image: checking stale guard");
            let result = kuru_platform::windows::process::current_image();
            println!(
                "{}",
                match result {
                    Ok(image) =>
                        serde_json::json!({"accepted":true,"identity":kuru_platform::fs::regular_file_info(image.file())?.identity.to_bytes()}),
                    Err(error) => serde_json::json!({"accepted":false,"error":error.to_string()}),
                }
            );
        }
        "console-check" => {
            use kuru_platform::windows::{
                console::{ConsoleModeGuard, virtual_terminal_output_enabled},
                process::StandardStream,
            };
            // This hidden-console parent deliberately has NUL standard streams.
            // Console allocation alone must not authorize ANSI on those streams.
            if ConsoleModeGuard::capture().is_ok()
                || [
                    StandardStream::Input,
                    StandardStream::Output,
                    StandardStream::Error,
                ]
                .into_iter()
                .any(virtual_terminal_output_enabled)
            {
                return Err("NUL standard streams were accepted as a console".into());
            }
            let input = File::options().read(true).write(true).open("CONIN$")?;
            let output = File::options().read(true).write(true).open("CONOUT$")?;
            let error = File::options().read(true).write(true).open("CONOUT$")?;
            let receipt = std::env::current_dir()?.join("console-public-result");
            let mut child = child_spec()?;
            child.args = vec![
                "console-inherited-check".into(),
                receipt.as_os_str().to_owned(),
            ];
            child.console = Console::Inherit;
            child.stdin = Stdio::Handle(input.into());
            child.stdout = Stdio::Handle(output.into());
            child.stderr = Stdio::Handle(error.into());
            let mut child = child.spawn().await?;
            let status = child.wait(Duration::from_secs(5)).await;
            if status.is_err() {
                child.terminate()?;
                child.wait(Duration::from_secs(5)).await?;
            }
            let status = status?;
            let report = fs::read_to_string(receipt)?;
            if !status.success() || report != "ok" {
                return Err(format!("inherited console public contracts failed: {report}").into());
            }
        }
        "console-inherited-check" => {
            let result = std::panic::catch_unwind(
                kuru_platform::windows::console::verify_private_console_fixture,
            );
            let report = match &result {
                Ok(Ok(())) => "ok".to_owned(),
                Ok(Err(error)) => error.to_string(),
                Err(panic) => panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| {
                        panic
                            .downcast_ref::<&str>()
                            .map(|message| (*message).to_owned())
                    })
                    .unwrap_or_else(|| "console assertion panicked without text".into()),
            };
            fs::write(&args[1], report)?;
            match result {
                Ok(result) => result?,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        "capture" => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            let report = serde_json::json!({
                "args": args[1..].iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
                "args_utf16": args[1..].iter().map(|arg| arg.encode_wide().collect::<Vec<_>>()).collect::<Vec<_>>(),
                "env": std::env::vars_os().map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())).collect::<std::collections::BTreeMap<_,_>>(),
                "wide_env": std::env::vars_os().map(|(key, value)| (key.to_string_lossy().into_owned(), value.encode_wide().collect::<Vec<_>>())).collect::<std::collections::BTreeMap<_,_>>(),
                "cwd": std::env::current_dir()?, "stdin": input,
            });
            eprintln!("fixture stderr");
            println!("{report}");
        }
        "idle" => {
            println!("ready");
            std::io::stdout().flush()?;
            std::future::pending::<()>().await;
        }
        "independent-leaf" => {
            let lock = File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&args[1])?;
            lock.lock()?;
            println!("independent-ready");
            std::io::stdout().flush()?;
            let deadline = Instant::now() + Duration::from_secs(20);
            while !std::path::Path::new(&args[2]).exists() {
                if Instant::now() >= deadline {
                    return Err("independent leaf release timed out".into());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        "independent-starter" => {
            let mut child = child_spec()?;
            child.args = vec!["independent-leaf".into(), args[1].clone(), args[2].clone()];
            child.lifetime = Lifetime::IndependentService;
            child.stdout = Stdio::Pipe;
            let mut child = child.spawn().await?;
            let mut lines =
                BufReader::new(child.take_stdout().ok_or("missing leaf output")?).lines();
            if lines.next_line().await?.as_deref() != Some("independent-ready") {
                return Err("independent leaf did not reach readiness".into());
            }
            println!("starter-exited");
            std::io::stdout().flush()?;
        }
        "capture-stall" => {
            println!("fixture stdout");
            eprintln!("fixture stderr");
            std::io::stdout().flush()?;
            std::io::stderr().flush()?;
            std::future::pending::<()>().await;
        }
        "capture-flood" => {
            let bytes = vec![b'x'; 1024 * 1024];
            match args.get(1).and_then(|arg| arg.to_str()) {
                Some("stdout") => std::io::stdout().write_all(&bytes)?,
                Some("stderr") => std::io::stderr().write_all(&bytes)?,
                _ => return Err("missing capture-flood stream".into()),
            }
            std::future::pending::<()>().await;
        }
        "leaf" => {
            let lock = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&args[1])?;
            lock.lock()?;
            println!("leaf-ready");
            std::io::stdout().flush()?;
            let start = Instant::now();
            while !std::path::Path::new(&args[2]).exists() {
                if start.elapsed() > Duration::from_secs(20) {
                    return Err("leaf release timed out".into());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        "tree" => {
            let root = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&args[1])?;
            root.lock()?;
            let mut child = Command::new(std::env::current_exe()?);
            child.arg("leaf").arg(&args[2]).arg(&args[3]);
            // This ordinary descendant must inherit the surrounding native Job.
            child.spawn()?;
            println!("root-ready");
            std::io::stdout().flush()?;
        }
        "owner" | "owner-startup" => {
            let mut child = child_spec()?;
            child.args = vec![
                if mode == "owner" { "tree" } else { "startup" }.into(),
                args[1].clone(),
                args[2].clone(),
                args[3].clone(),
            ];
            child.stdout = Stdio::Pipe;
            let mut child = child.spawn().await?;
            let mut lines =
                BufReader::new(child.take_stdout().ok_or("missing child output")?).lines();
            for _ in 0..if mode == "owner" { 2 } else { 1 } {
                lines.next_line().await?.ok_or("missing tree readiness")?;
            }
            println!("owned-tree-ready");
            std::io::stdout().flush()?;
            std::future::pending::<()>().await;
        }
        "startup" => {
            let lock = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&args[1])?;
            lock.lock()?;
            println!("child-startup");
            std::io::stdout().flush()?;
            while !std::path::Path::new(&args[2]).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            fs::write(&args[3], b"work-started")?;
        }
        "rendezvous" => {
            let mut channel = pipe::connect(&args[1], Duration::from_secs(5)).await?;
            channel.write_all(b"connected").await?;
            channel.flush().await?;
            let mut bytes = Vec::new();
            channel.read_to_end(&mut bytes).await?;
            let destination = std::path::Path::new(&args[2]);
            let candidate = destination.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .open(&candidate)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(candidate, destination)?;
        }
        "rendezvous-stall" => {
            let mut channel = pipe::connect(&args[1], Duration::from_secs(5)).await?;
            channel.write_all(b"connected").await?;
            channel.flush().await?;
            std::future::pending::<()>().await;
        }
        "rendezvous-partial-frame" => {
            let mut channel = pipe::connect(&args[1], Duration::from_secs(5)).await?;
            channel.write_all(b"{\"frame\":").await?;
            channel.flush().await?;
            println!("partial-frame-sent");
            std::io::stdout().flush()?;
            let mut byte = [0];
            if channel.read(&mut byte).await? != 0 {
                return Err("partial-frame peer received unexpected payload instead of EOF".into());
            }
            channel.close(Duration::from_secs(5)).await?;
            println!("partial-frame-eof");
            std::io::stdout().flush()?;
        }
        "duplex" => {
            let mut channel = pipe::connect(&args[1], Duration::from_secs(5)).await?;
            let mut input = vec![0; 8 * 1024 * 1024];
            channel.read_exact(&mut input).await?;
            if input.iter().any(|byte| *byte != b'x') {
                return Err("duplex payload mismatch".into());
            }
            channel.write_all(b"duplex-ok").await?;
            channel.flush().await?;
        }
        "console-owner" => {
            let mut child = child_spec()?;
            child.args = vec!["idle".into()];
            child.console = Console::NewProcessGroup;
            child.stdout = Stdio::Pipe;
            let mut child = child.spawn().await?;
            BufReader::new(child.take_stdout().ok_or("missing child output")?)
                .lines()
                .next_line()
                .await?
                .ok_or("missing group readiness")?;
            child.interrupt()?;
            child.wait(Duration::from_secs(5)).await?;
            println!("group-stopped");
        }
        "trusted-owner" => {
            let listener = pipe::PrivateListener::bind()?;
            let mut child = child_spec()?;
            child.args = vec![
                "rendezvous".into(),
                listener.address().to_owned(),
                args[1].clone(),
            ];
            child.lifetime = Lifetime::TrustedSupervisor;
            let child = child.spawn().await?;
            let mut channel = listener.accept(&child, Duration::from_secs(5)).await?;
            let mut connected = [0; 9];
            channel.read_exact(&mut connected).await?;
            channel.write_all(b"parent-lifetime").await?;
            channel.flush().await?;
            println!("trusted-ready");
            std::io::stdout().flush()?;
            std::future::pending::<()>().await;
        }
        _ => return Err("unknown fixture mode".into()),
    }
    Ok(())
}
