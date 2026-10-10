#![cfg(feature = "tooling")]

use kuru_delivery::{archive::MAX_ARCHIVE_BYTES, command::Command, signing};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
#[cfg(any(target_os = "macos", windows))]
use std::path::Path;
use std::{ffi::OsStr, fs};

#[path = "support/files.rs"]
mod files;

const LINUX: &str = "x86_64-unknown-linux-gnu";

async fn run(mut command: Command) -> std::process::Output {
    kuru_delivery::command::bounded_output(
        &mut command,
        std::time::Duration::from_secs(30),
        64 * 1024,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn fake_signer_replaces_private_copy_without_changing_hardlinked_build_input() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("build executable");
    let link = root.path().join("other Cargo link");
    let original = b"the original build executable";
    files::executable(&source, original);
    fs::hard_link(&source, &link).unwrap();
    let identity = files::identity(&source);
    let output = root.path().join("private signing output");
    let copy = signing::prepare(&source, LINUX, &output).unwrap();
    assert_ne!(files::identity(&copy), identity);
    assert_eq!(fs::read(&copy).unwrap(), original);
    let copy_identity = files::identity(&copy);
    let directory = Directory::open(&output, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
    // The signer uses a replacement inode, as codesign may do. Its temporary
    // output is a fresh checked private file, never another link to the input.
    drop(directory.create_new(OsStr::new("replacement")).unwrap());
    #[cfg(unix)]
    let command = {
        let mut command = Command::new("/bin/sh");
        command.args(["-e", "-c", "printf '%s' 'signed fixture bytes' > \"$1/replacement\"; chmod 700 \"$1/replacement\"; mv -f \"$1/replacement\" \"$1/kuru\"", "fake-signer"]).arg(&output);
        command
    };
    #[cfg(windows)]
    let command = {
        let program = kuru_platform::windows::process::system_directory()
            .unwrap()
            .join("WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(program);
        command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; $replacement = [IO.Path]::Combine($env:KURU_FAKE_SIGNING_OUTPUT, 'replacement'); $binary = [IO.Path]::Combine($env:KURU_FAKE_SIGNING_OUTPUT, 'kuru'); [IO.File]::WriteAllBytes($replacement, [Text.Encoding]::UTF8.GetBytes('signed fixture bytes')); [IO.File]::Delete($binary); [IO.File]::Move($replacement, $binary)"])
            .env_remove("PSModulePath")
            .env("KURU_FAKE_SIGNING_OUTPUT", &output);
        command
    };
    let result = run(command).await;
    assert!(result.status.success(), "{result:?}");
    assert_ne!(files::identity(&copy), copy_identity);
    assert_eq!(fs::read(&copy).unwrap(), b"signed fixture bytes");
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&link).unwrap(), original);
    assert_eq!(files::identity(&source), identity);
    assert_eq!(files::identity(&link), identity);
    signing::verify(&copy, LINUX, "").await.unwrap();
    assert!(signing::prepare(&source, LINUX, &output).is_err());
    assert_eq!(fs::read(&copy).unwrap(), b"signed fixture bytes");
    assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
}

#[test]
fn unsafe_existing_destinations_are_never_replaced() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    files::executable(&source, b"original");
    for kind in ["file", "directory", "hardlink", "symlink"] {
        let path = root.path().join(kind);
        let directory = Directory::ensure_private(&path).unwrap();
        let destination = path.join("kuru");
        match kind {
            "file" => {
                use std::io::Write;
                directory
                    .create_new(OsStr::new("kuru"))
                    .unwrap()
                    .write_all(b"existing")
                    .unwrap();
            }
            "directory" => {
                directory
                    .create_private_directory(OsStr::new("kuru"))
                    .unwrap();
            }
            "hardlink" => {
                fs::hard_link(&source, &destination).unwrap();
            }
            "symlink" => {
                files::symlink(&source, &destination).unwrap();
            }
            _ => unreachable!(),
        }
        let metadata = fs::symlink_metadata(&destination).unwrap();
        assert!(signing::prepare(&source, LINUX, &path).is_err(), "{kind}");
        assert_eq!(
            fs::symlink_metadata(&destination).unwrap().file_type(),
            metadata.file_type()
        );
        assert_eq!(fs::read(&source).unwrap(), b"original");
        if kind == "file" {
            assert_eq!(fs::read(&destination).unwrap(), b"existing");
        }
        assert_eq!(
            fs::read_dir(&path).unwrap().count(),
            1,
            "failed stage leaked for {kind}"
        );
    }
    let public = root.path().join("nonprivate output");
    fs::create_dir(&public).unwrap();
    files::mode(&public, 0o755);
    assert!(signing::prepare(&source, LINUX, &public).is_err());
    assert_eq!(fs::read_dir(public).unwrap().count(), 0);
    let linked = root.path().join("linked output root");
    files::symlink(root.path().join("file"), &linked).unwrap();
    assert!(signing::prepare(&source, LINUX, &linked).is_err());
}

#[test]
fn invalid_inputs_fail_before_creating_signing_state() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let output = root.path().join("output");
    files::executable(&source, b"");
    assert!(signing::prepare(&source, LINUX, &output).is_err());
    assert!(!output.exists());
    let file = fs::OpenOptions::new().write(true).open(&source).unwrap();
    file.set_len(MAX_ARCHIVE_BYTES as u64 + 1).unwrap();
    drop(file);
    assert!(signing::prepare(&source, LINUX, &output).is_err());
    assert!(!output.exists());
    files::executable(&source, b"valid bounded bytes");
    assert!(signing::prepare(&source, "unknown-target", &output).is_err());
    let linked = root.path().join("linked input");
    files::symlink(&source, &linked).unwrap();
    assert!(signing::prepare(&linked, LINUX, &output).is_err());
    assert!(signing::prepare(root.path(), LINUX, &output).is_err());
    assert!(!output.exists());
}

#[tokio::test]
async fn verification_rejects_unsafe_replacements_and_invalid_publisher_configuration() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    files::executable(&source, b"bounded source");
    let output = root.path().join("output");
    let copy = signing::prepare(&source, LINUX, &output).unwrap();
    assert!(signing::verify(&copy, "unknown", "").await.is_err());
    assert!(
        signing::verify(&copy, "aarch64-apple-darwin", "bad-team")
            .await
            .is_err()
    );
    assert!(
        signing::verify(&copy, "x86_64-pc-windows-msvc", "")
            .await
            .is_err()
    );
    let windows = signing::prepare(
        &source,
        "x86_64-pc-windows-msvc",
        &root.path().join("windows"),
    )
    .unwrap();
    assert!(
        signing::verify(&windows, "x86_64-pc-windows-msvc", "")
            .await
            .is_err()
    );
    assert!(
        signing::verify(&windows, "x86_64-pc-windows-msvc", "CN=Fixture\nInjected")
            .await
            .is_err()
    );
    let other = output.join("other link");
    fs::hard_link(&copy, &other).unwrap();
    assert!(signing::verify(&copy, LINUX, "").await.is_err());
    fs::remove_file(other).unwrap();
    fs::remove_file(&copy).unwrap();
    files::symlink(&source, &copy).unwrap();
    assert!(signing::verify(&copy, LINUX, "").await.is_err());
    fs::remove_file(&copy).unwrap();
    // A signer must preserve private payload grants even if it replaces the inode.
    files::executable(&copy, b"public replacement");
    files::mode(&copy, 0o755);
    assert!(signing::verify(&copy, LINUX, "").await.is_err());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn actual_ad_hoc_macos_signature_is_structurally_valid_but_not_a_publisher_signature() {
    let root = tempfile::tempdir().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    let original = fs::read(binary).unwrap();
    let signed =
        signing::prepare(binary, "aarch64-apple-darwin", &root.path().join("signed")).unwrap();
    let mut sign = Command::new("/usr/bin/codesign");
    sign.args([
        "--force",
        "--sign",
        "-",
        "--options",
        "runtime",
        "--timestamp=none",
    ])
    .arg(&signed);
    let result = run(sign).await;
    assert!(result.status.success(), "{result:?}");
    let mut check = Command::new("/usr/bin/codesign");
    check.args(["--verify", "--strict"]).arg(&signed);
    let result = run(check).await;
    assert!(result.status.success(), "{result:?}");
    let error = signing::verify(&signed, "aarch64-apple-darwin", "ABCDEFGHIJ")
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("Developer ID signature verification failed"),
        "{error:#}"
    );
    assert_eq!(fs::read(binary).unwrap(), original);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn notarization_script_accepts_only_accepted_results_and_cleans_every_temporary_identity() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("support/sign-macos.sh");
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("fake signing tools");
    fs::create_dir(&tools).unwrap();
    let wrappers = [
        (
            "security",
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> "$KURU_FAKE_SIGNING_LOG"
case "$1" in
  create-keychain)
    printf '%s\n' "$4" > "$KURU_FAKE_KEYCHAIN_PATH"
    printf '%s' 'fixture keychain marker' > "$4"
    ;;
  delete-keychain)
    test -f "$2"
    rm -- "$2"
    ;;
  unlock-keychain|import|set-key-partition-list) ;;
  *) exit 91 ;;
esac
"#,
        ),
        (
            "codesign",
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "codesign:$1" >> "$KURU_FAKE_SIGNING_LOG"
case "$1" in
  --force)
    test "$2" = '--options'
    test "$3" = 'runtime'
    test "$4" = '--timestamp'
    test "${!#}" = "$KURU_SIGNING_BINARY"
    printf '%s' 'fake signed executable' > "$KURU_SIGNING_BINARY"
    ;;
  --verify) test "${!#}" = "$KURU_SIGNING_BINARY" ;;
  *) exit 92 ;;
esac
"#,
        ),
        (
            "ditto",
            r#"#!/bin/bash
set -euo pipefail
test "$1" = '-c'
test "$2" = '-k'
test "$3" = '--keepParent'
test "$4" = "$KURU_SIGNING_BINARY"
printf '%s\n' 'ditto' >> "$KURU_FAKE_SIGNING_LOG"
cp -- "$4" "$5"
"#,
        ),
        (
            "xcrun",
            r#"#!/bin/bash
set -euo pipefail
test "$1" = 'notarytool'
test "$2" = 'submit'
test -f "$3"
printf '%s\n' 'notarytool' >> "$KURU_FAKE_SIGNING_LOG"
case "$KURU_FAKE_NOTARY_RESULT" in
  Accepted) printf '%s\n' '{"status":"Accepted","id":"fixture-notary-request"}' ;;
  Invalid) printf '%s\n' '{"status":"Invalid","id":"fixture-notary-request"}' ;;
  malformed) printf '%s\n' '{broken json' ;;
  nonzero) printf '%s\n' '{"status":"Accepted","id":"fixture-notary-request"}'; exit 42 ;;
  *) exit 93 ;;
esac
"#,
        ),
    ];
    for (name, body) in wrappers {
        files::executable(&tools.join(name), body.as_bytes());
    }
    let mut paths = vec![tools];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let source = root.path().join("original Cargo executable");
    let link = root.path().join("original Cargo hardlink");
    let original = b"original executable bytes";
    files::executable(&source, original);
    fs::hard_link(&source, &link).unwrap();
    let identity = files::identity(&source);
    let sentinel = root.path().join("unrelated file");
    fs::write(&sentinel, b"unrelated original").unwrap();
    for result in ["Accepted", "Invalid", "malformed", "nonzero"] {
        let fixture = root.path().join(result);
        let directory = Directory::ensure_private(&fixture).unwrap();
        let scratch = directory
            .create_private_directory(OsStr::new("scratch"))
            .unwrap();
        let log = fixture.join("operations");
        let keychain_path = fixture.join("keychain-path");
        let summary = fixture.join("summary");
        let signed =
            signing::prepare(&source, "aarch64-apple-darwin", &fixture.join("signed")).unwrap();
        let mut command = Command::new("/bin/bash");
        command
            .arg(&script)
            .env_clear()
            .env("PATH", &path)
            .env("RUNNER_TEMP", scratch.path())
            .env("TMPDIR", scratch.path())
            .env("KURU_SIGNING_BINARY", &signed)
            .env("KURU_FAKE_SIGNING_LOG", &log)
            .env("KURU_FAKE_KEYCHAIN_PATH", &keychain_path)
            .env("KURU_FAKE_NOTARY_RESULT", result)
            .env("GITHUB_STEP_SUMMARY", &summary)
            .env("MACOS_SIGNING_P12_BASE64", "Zml4dHVyZSBjZXJ0aWZpY2F0ZQ==")
            .env("MACOS_SIGNING_P12_PASSWORD", "fake-certificate-password")
            .env(
                "MACOS_SIGNING_IDENTITY",
                "Developer ID Application: Fixture (ABCDEFGHIJ)",
            )
            .env("APPLE_NOTARY_KEY_P8", "fixture-only-notary-key")
            .env("APPLE_NOTARY_KEY_ID", "FIXTUREKEY")
            .env("APPLE_NOTARY_ISSUER_ID", "fixture-issuer");
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let output = run(command).await;
        assert_eq!(
            output.status.success(),
            result == "Accepted",
            "{result}: {output:?}"
        );
        if result == "nonzero" {
            assert_eq!(output.status.code(), Some(42));
        }
        let observed = fs::read_to_string(&log).unwrap();
        assert_eq!(
            observed.lines().collect::<Vec<_>>(),
            [
                "create-keychain",
                "unlock-keychain",
                "import",
                "set-key-partition-list",
                "codesign:--force",
                "codesign:--verify",
                "ditto",
                "notarytool",
                "delete-keychain"
            ],
            "{result} skipped signing or cleanup"
        );
        let keychain = fs::read_to_string(&keychain_path).unwrap();
        let keychain = Path::new(keychain.trim_end());
        assert!(keychain.starts_with(scratch.path()));
        assert!(!keychain.exists(), "{result} retained temporary keychain");
        assert!(
            !keychain.parent().unwrap().exists(),
            "{result} retained private key material"
        );
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
        assert_eq!(fs::read(&signed).unwrap(), b"fake signed executable");
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(fs::read(&link).unwrap(), original);
        assert_eq!(files::identity(&source), identity);
        assert_eq!(files::identity(&link), identity);
        assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated original");
        if result == "Accepted" {
            assert_eq!(
                fs::read_to_string(summary).unwrap(),
                "Apple notarization accepted: fixture-notary-request\n"
            );
        } else {
            assert!(!summary.exists(), "{result} emitted accepted evidence");
        }
    }
}

#[cfg(windows)]
#[tokio::test]
async fn native_windows_verifier_rejects_unsigned_pe_for_each_release_architecture() {
    let root = tempfile::tempdir().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
        let copy = signing::prepare(binary, target, &root.path().join(target)).unwrap();
        let error = signing::verify(&copy, target, "CN=Fixture Publisher")
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("Authenticode signature is not valid"),
            "{error:#}"
        );
    }
}
