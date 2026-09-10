//! Native compiled-fixture helpers; no production configuration or fault switches.
use anyhow::{Context, Result, ensure};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy},
    windows::pipe::Pipe,
};
use std::{ffi::OsString, path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub fn environment() -> Result<Vec<(OsString, OsString)>> {
    let system = kuru_platform::windows::process::system_directory()?;
    let root = system.parent().context("system directory has no parent")?;
    let mut environment = vec![
        ("SystemRoot".into(), root.as_os_str().into()),
        ("PATH".into(), system.into_os_string()),
    ];
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        environment.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    Ok(environment)
}

pub async fn partial_readiness(
    options: crate::server::ServerOptions,
    observer: &mut Pipe,
) -> Result<()> {
    crate::server::windows_fixture::partial_readiness(options, observer).await
}

struct EngineOwner {
    child: Option<crate::engine::Child>,
    directory: Option<Directory>,
}

impl Drop for EngineOwner {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let directory = self.directory.take();
        let _ = child.kill();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            let mut warned = false;
            while !matches!(child.try_wait(), Ok(Some(_))) {
                if !warned && std::time::Instant::now() >= deadline {
                    eprintln!("engine fixture cleanup delayed; retaining its tree and directory");
                    warned = true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            drop(directory);
        });
    }
}

/// Run the real engine adapter against a compiled root and descendant whose
/// registered console-break handlers deliberately keep their process alive.
/// The caller must already own a private console, as the real supervisor does.
pub async fn forced_engine_cleanup(root: &Path, fixture: &Path, observer: &mut Pipe) -> Result<()> {
    let directory = Directory::open(root, Privacy::OwnerOnly, NameRetention::Movable)?;
    let home = root.join("engine home café 東京");
    crate::provision::prepare_private_home(&home)?;
    let extra = std::env::var_os("LLVM_PROFILE_FILE")
        .map(|profile| vec![("LLVM_PROFILE_FILE".into(), profile)])
        .unwrap_or_default();
    let child = crate::engine::spawn(
        fixture,
        &home,
        root,
        vec!["--stubborn-root".into(), root.as_os_str().into()],
        extra,
        true,
    )
    .await?;
    let mut owner = EngineOwner {
        child: Some(child),
        directory: Some(directory),
    };
    let child = owner.child.as_mut().unwrap();
    let mut stdout = child.stdout().context("engine fixture has no output")?;
    let mut ready = [0; 6];
    tokio::time::timeout(Duration::from_secs(8), stdout.read_exact(&mut ready)).await??;
    ensure!(&ready == b"READY\n", "stubborn descendant was not ready");
    let lock = owner
        .directory
        .as_ref()
        .unwrap()
        .lock_file(std::ffi::OsStr::new("descendant.lock"))?;
    ensure!(
        matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "descendant did not retain its actual file lock"
    );
    child
        .stop(Duration::from_secs(2), Duration::from_secs(5))
        .await?;
    let status = child
        .try_wait()?
        .context("engine stop returned before Job quiescence")?;
    ensure!(!status.success(), "stubborn fixture exited cooperatively");
    let mut output = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        (&mut stdout).take(1024).read_to_end(&mut output),
    )
    .await??;
    let output = String::from_utf8(output)?;
    ensure!(
        output.len() < 1024,
        "engine fixture output exceeded its bounded EOF observation"
    );
    ensure!(
        output.contains("BREAK-root\n"),
        "root did not observe the graceful signal"
    );
    ensure!(
        output.contains("BREAK-leaf\n"),
        "descendant did not observe the graceful signal"
    );
    lock.try_lock()?;
    stdout.close(Duration::from_secs(3)).await?;
    owner.child.take();
    observer.write_all(b"READY\n").await?;
    observer.flush().await?;
    Ok(())
}
