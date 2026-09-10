#![cfg(all(windows, feature = "tooling"))]

use kuru_delivery::{archive::digest, command::BlockingCommand as Command};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy, regular_file_info, require_private},
    windows::process::{NativeSpawnSpec, Stdio},
};
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(30);

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    current: PathBuf,
    candidate: PathBuf,
    cache: PathBuf,
    marker: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("ordinary mise bin λ");
        fs::create_dir(&directory).unwrap();
        let current = directory.join("kuru.exe");
        let original = fs::read(env!("CARGO_BIN_EXE_kuru-delivery-fixture")).unwrap();
        fs::write(&current, &original).unwrap();
        let candidate = root.path().join("candidate.exe");
        let mut replacement = original.clone();
        replacement.extend_from_slice(b"KURU_CANDIDATE_MARKER");
        fs::write(&candidate, &replacement).unwrap();
        let cache = root.path().join("private helper cache");
        let marker = root.path().join("candidate-executed");
        Self {
            _root: root,
            directory,
            current,
            candidate,
            cache,
            marker,
            original,
            replacement,
        }
    }

    fn spawn(&self) -> NativeSpawnSpec {
        let mut spec = NativeSpawnSpec::new(self.current.clone(), self.directory.clone());
        spec.args = vec![
            "update".into(),
            self.candidate.clone().into(),
            self.cache.clone().into(),
        ];
        spec.environment = vec![("KURU_EXECUTION_MARKER".into(), self.marker.clone().into())];
        spec.stdin = Stdio::Pipe;
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        spec
    }

    fn receipt(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.directory.join(".kuru-update/receipt.json")).unwrap())
            .unwrap()
    }
}

async fn line(pipe: &mut kuru_platform::windows::pipe::Pipe) -> String {
    tokio::time::timeout(TIMEOUT, async {
        let mut bytes = Vec::new();
        loop {
            let byte = pipe.read_u8().await?;
            if byte == b'\n' {
                break;
            }
            if bytes.len() >= 65536 {
                return Err(io::Error::other("fixture output overflow"));
            }
            bytes.push(byte);
        }
        String::from_utf8(bytes).map_err(io::Error::other)
    })
    .await
    .unwrap()
    .unwrap()
}

fn assert_private_receipt(fixture: &Fixture) {
    let directory = Directory::open(
        &fixture.directory.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )
    .unwrap();
    for name in ["receipt.json", "install.lock"] {
        require_private(&directory.read(OsStr::new(name)).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn loaded_image_acknowledges_exact_new_bytes_while_old_process_is_still_alive() {
    let fixture = Fixture::new();
    let original_file = fs::File::open(&fixture.current).unwrap();
    let original_identity = regular_file_info(&original_file).unwrap().identity;
    drop(original_file);
    let mut child = fixture.spawn().spawn().await.unwrap();
    let mut input = child.take_stdin().unwrap();
    let mut output = child.take_stdout().unwrap();
    let ack: serde_json::Value = serde_json::from_str(&line(&mut output).await).unwrap();
    assert_eq!(ack["installed"], fixture.current.to_string_lossy().as_ref());
    assert_eq!(ack["cleanup_pending"], true);
    assert!(
        child.try_wait().unwrap().is_none(),
        "old process exited before observer checked publication"
    );
    assert_eq!(fs::read(&fixture.current).unwrap(), fixture.replacement);
    assert!(
        !fixture.marker.exists(),
        "candidate ran during verification or helper execution"
    );
    let receipt = fixture.receipt();
    let displaced = fixture
        .directory
        .join(receipt["displaced"].as_str().unwrap());
    assert_eq!(
        regular_file_info(&fs::File::open(&displaced).unwrap())
            .unwrap()
            .identity,
        original_identity
    );
    assert_eq!(fs::read(&displaced).unwrap(), fixture.original);
    assert_private_receipt(&fixture);
    let helper = Path::new(receipt["helper"].as_str().unwrap());
    assert_eq!(fs::read(helper).unwrap(), fixture.original);
    assert!(
        helper
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&digest(&fixture.original))
    );
    require_private(&fs::File::open(helper).unwrap()).unwrap();
    input.write_all(b"x").await.unwrap();
    input.flush().await.unwrap();
    input.close(TIMEOUT).await.unwrap();
    assert!(child.wait(TIMEOUT).await.unwrap().success());
    output.close(TIMEOUT).await.unwrap();
    // The cached current-version image is deliberately persistent. A fresh
    // public invocation runs the new image only after verified acknowledgment.
    assert!(helper.is_file());
    let result = Command::new(&fixture.current)
        .arg("--version")
        .env("KURU_EXECUTION_MARKER", &fixture.marker)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(fixture.marker.is_file());
}

#[tokio::test]
async fn corrupt_existing_trusted_helper_is_never_executed_and_preserves_current_image() {
    let fixture = Fixture::new();
    let cache = Directory::ensure_private(&fixture.cache).unwrap();
    let name = format!("x86_64-pc-windows-msvc-{}.exe", digest(&fixture.original));
    let mut wrong = cache.create_new(OsStr::new(&name)).unwrap();
    std::io::Write::write_all(&mut wrong, b"not the trusted current image").unwrap();
    drop(wrong);
    let mut child = fixture.spawn().spawn().await.unwrap();
    let mut error = child.take_stderr().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(TIMEOUT, error.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(!child.wait(TIMEOUT).await.unwrap().success());
    error.close(TIMEOUT).await.unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("cached trusted helper is corrupt")
    );
    assert_eq!(fs::read(&fixture.current).unwrap(), fixture.original);
    assert!(!fixture.marker.exists());
    assert!(!fixture.directory.join(".kuru-update").exists());
}
