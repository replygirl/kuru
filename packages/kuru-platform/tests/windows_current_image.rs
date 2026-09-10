#![cfg(all(windows, feature = "test-support"))]

use kuru_platform::{
    fs::{Directory, NameRetention, Privacy, Publication, regular_file_info},
    windows::{
        pipe::Pipe,
        process::{NativeSpawnSpec, Stdio},
    },
};
use std::{ffi::OsStr, fs, io, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(10);

fn require(condition: bool, message: impl Into<String>) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message.into()))
    }
}

async fn line(pipe: &mut Pipe, stage: &str) -> io::Result<String> {
    tokio::time::timeout(TIMEOUT, async {
        let mut output = Vec::new();
        loop {
            let value = pipe.read_u8().await?;
            if value == b'\n' {
                break;
            }
            if output.len() == 4096 {
                return Err(io::Error::other(
                    "image fixture acknowledgment exceeds bound",
                ));
            }
            output.push(value);
        }
        String::from_utf8(output).map_err(io::Error::other)
    })
    .await
    .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, format!("{stage}: {error}")))?
    .map_err(|error| io::Error::new(error.kind(), format!("{stage}: {error}")))
}

#[tokio::test]
async fn current_image_is_pinned_while_reading_and_rejects_rebound_identical_path_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("current image λ.exe");
    let bytes = fs::read(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")).unwrap();
    fs::write(&path, &bytes).unwrap();
    let parent = Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap();
    // Keep the ancestor pinned, but do not let this test's source read itself
    // block rename: the child guard must be the only non-delete-sharing file.
    let source = Directory::open(root.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
    assert_eq!(parent.identity(), source.identity());
    let original = source.read(OsStr::new("current image λ.exe")).unwrap();
    let identity = regular_file_info(&original).unwrap().identity;
    drop(original);
    let mut spec = NativeSpawnSpec::new(path.clone(), root.path().to_owned());
    spec.args = vec!["current-image".into()];
    for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(key) {
            spec.environment.push((key.into(), value));
        }
    }
    spec.stdin = Stdio::Pipe;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec.spawn().await.unwrap();
    let mut input = child.take_stdin().unwrap();
    let mut output = child.take_stdout().unwrap();
    let mut errors = child.take_stderr().unwrap();
    let exercise = async {
        let result: io::Result<()> = async {
            let held: serde_json::Value =
                serde_json::from_str(&line(&mut output, "initial guard").await?)?;
            require(
                held["ready"] == "held",
                format!("initial guard acknowledgment: {held}"),
            )?;
            require(
                held["identity"] == serde_json::json!(identity.to_bytes()),
                format!("initial guard identity: {held}"),
            )?;
            let original = source.read(OsStr::new("current image λ.exe"))?;
            require(
                parent
                    .rename_file(
                        &source,
                        OsStr::new("current image λ.exe"),
                        &original,
                        OsStr::new("displaced.exe"),
                        Publication::New,
                    )
                    .is_err(),
                "current-image trust guard allowed replacement while copying",
            )?;
            require(fs::read(&path)? == bytes, "held image bytes changed")?;
            require(
                regular_file_info(&source.read(OsStr::new("current image λ.exe"))?)?.identity
                    == identity,
                "rejected move changed the original image identity",
            )?;
            require(
                source
                    .read(OsStr::new("displaced.exe"))
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound),
                "rejected move created a displaced image",
            )?;
            input.write_all(b"x").await?;
            input.flush().await?;
            let released = line(&mut output, "guard release").await?;
            require(
                released.trim() == "released",
                format!("guard release acknowledgment: {released}"),
            )?;
            parent
                .rename_file(
                    &source,
                    OsStr::new("current image λ.exe"),
                    &original,
                    OsStr::new("displaced.exe"),
                    Publication::New,
                )
                .map_err(io::Error::other)?;
            require(
                regular_file_info(&parent.read(OsStr::new("displaced.exe"))?)?.identity == identity,
                "successful move changed the original image identity",
            )?;
            require(
                fs::read(root.path().join("displaced.exe"))? == bytes,
                "successful move changed the original image bytes",
            )?;
            require(
                source
                    .read(OsStr::new("current image λ.exe"))
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound),
                "successful move retained the original image name",
            )?;
            drop(original);
            fs::write(&path, &bytes)?;
            let replacement = parent.read(OsStr::new("current image λ.exe"))?;
            let replacement_id = regular_file_info(&replacement)?.identity;
            require(
                identity != replacement_id,
                "replacement retained the old identity",
            )?;
            drop(replacement);
            require(child.try_wait()?.is_none(), "old mapped image exited")?;
            input.write_all(b"x").await?;
            input.flush().await?;
            let result: serde_json::Value =
                serde_json::from_str(&line(&mut output, "stale guard").await?)?;
            require(
                result["accepted"] == false,
                format!("mapped-file name failed to distinguish a stale loaded image: initial={held}; after rename={result}"),
            )?;
            require(
                result["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("loaded image")),
                format!("stale guard error: {result}"),
            )?;
            require(fs::read(&path)? == bytes, "replacement bytes changed")?;
            require(
                regular_file_info(&parent.read(OsStr::new("current image λ.exe"))?)?.identity
                    == replacement_id,
                "replacement identity changed",
            )?;
            require(
                held["diagnostic_only"] == false && held["initial_error"].is_null(),
                format!("initial current-image trust failed (diagnostic fallback never satisfies acceptance): initial={held}; after rename={result}"),
            )?;
            Ok(())
        }
        .await;
        // Preserve the pre-cleanup status in diagnostics. A failed assertion must
        // still terminate/reap the owned tree and observe closure of every pipe.
        let before_cleanup = child.try_wait();
        let terminated = if result.is_err() {
            child.terminate()
        } else {
            Ok(())
        };
        let input_closed = input.close(TIMEOUT).await;
        let output_closed = output.close(TIMEOUT).await;
        let status = child.wait(TIMEOUT).await;
        let forced_cleanup = if status.is_err() {
            let terminated = child.terminate();
            let waited = child.wait(TIMEOUT).await;
            Some((terminated, waited))
        } else {
            None
        };
        (
            result,
            before_cleanup,
            terminated,
            input_closed,
            output_closed,
            status,
            forced_cleanup,
        )
    };
    let mut stderr = Vec::new();
    let drain = async {
        let read = tokio::time::timeout(
            TIMEOUT * 6,
            (&mut errors).take(65537).read_to_end(&mut stderr),
        )
        .await;
        let closed = errors.close(TIMEOUT).await;
        (read, closed)
    };
    let (exercise, drain) = tokio::join!(exercise, drain);
    assert!(
        exercise.0.is_ok()
            && exercise.2.is_ok()
            && exercise.3.is_ok()
            && exercise.4.is_ok()
            && exercise.5.as_ref().is_ok_and(|status| status.success())
            && drain.0.as_ref().is_ok_and(|result| result.is_ok())
            && drain.1.is_ok()
            && stderr.len() <= 65536,
        "image fixture result/cleanup: {exercise:?}; stderr drain: {drain:?}\nstderr: {}",
        String::from_utf8_lossy(&stderr)
    );
}
