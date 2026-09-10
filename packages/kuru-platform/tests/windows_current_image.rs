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

async fn line(pipe: &mut Pipe) -> String {
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
    .expect("native image fixture did not acknowledge")
    .unwrap()
}

#[tokio::test]
async fn current_image_is_pinned_while_reading_and_rejects_rebound_identical_path_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("current image λ.exe");
    let bytes = fs::read(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")).unwrap();
    fs::write(&path, &bytes).unwrap();
    let parent = Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap();
    let original = parent.read(OsStr::new("current image λ.exe")).unwrap();
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
    let held: serde_json::Value = serde_json::from_str(&line(&mut output).await).unwrap();
    assert_eq!(held["ready"], "held");
    assert_eq!(held["identity"], serde_json::json!(identity.to_bytes()));
    let original = parent.read(OsStr::new("current image λ.exe")).unwrap();
    assert!(
        parent
            .rename_file(
                &parent,
                OsStr::new("current image λ.exe"),
                &original,
                OsStr::new("displaced.exe"),
                Publication::New,
            )
            .is_err(),
        "current-image trust guard allowed replacement while copying"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    input.write_all(b"x").await.unwrap();
    input.flush().await.unwrap();
    assert_eq!(line(&mut output).await.trim(), "released");
    parent
        .rename_file(
            &parent,
            OsStr::new("current image λ.exe"),
            &original,
            OsStr::new("displaced.exe"),
            Publication::New,
        )
        .unwrap();
    drop(original);
    fs::write(&path, &bytes).unwrap();
    let replacement = parent.read(OsStr::new("current image λ.exe")).unwrap();
    let replacement_id = regular_file_info(&replacement).unwrap().identity;
    assert_ne!(identity, replacement_id);
    drop(replacement);
    assert!(
        child.try_wait().unwrap().is_none(),
        "old mapped image exited"
    );
    input.write_all(b"x").await.unwrap();
    input.flush().await.unwrap();
    let result: serde_json::Value = serde_json::from_str(&line(&mut output).await).unwrap();
    assert_eq!(
        result["accepted"], false,
        "mapped-file name failed to distinguish a stale loaded image: {result}"
    );
    assert!(result["error"].as_str().unwrap().contains("loaded image"));
    input.close(TIMEOUT).await.unwrap();
    output.close(TIMEOUT).await.unwrap();
    let mut stderr = Vec::new();
    tokio::time::timeout(TIMEOUT, (&mut errors).take(65536).read_to_end(&mut stderr))
        .await
        .unwrap()
        .unwrap();
    let status = child.wait(TIMEOUT).await.unwrap();
    errors.close(TIMEOUT).await.unwrap();
    assert!(
        status.success(),
        "{status}: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        regular_file_info(&parent.read(OsStr::new("current image λ.exe")).unwrap())
            .unwrap()
            .identity,
        replacement_id
    );
}
