//! Native fixture executable; never a product entrypoint.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(std::time::Duration::from_secs(30), run()).await?
}

#[cfg(windows)]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    use kuru_platform::windows::{
        pipe,
        process::{Console, Lifetime, NativeSpawnSpec, Stdio},
    };
    use std::{
        fs::{self, File},
        io::{Read, Write},
        os::windows::ffi::OsStrExt,
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
            let image = kuru_platform::windows::process::current_image()?;
            let identity = kuru_platform::fs::regular_file_info(image.file())?
                .identity
                .to_bytes();
            println!(
                "{}",
                serde_json::json!({"ready":"held", "identity":identity})
            );
            std::io::stdout().flush()?;
            let mut byte = [0];
            std::io::stdin().read_exact(&mut byte)?;
            drop(image);
            println!("released");
            std::io::stdout().flush()?;
            std::io::stdin().read_exact(&mut byte)?;
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
            kuru_platform::windows::console::verify_private_console_fixture()?;
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
            tokio::time::sleep(Duration::from_secs(30)).await;
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
            tokio::time::sleep(Duration::from_secs(30)).await;
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
            tokio::time::sleep(Duration::from_secs(30)).await;
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
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        _ => return Err("unknown fixture mode".into()),
    }
    Ok(())
}
