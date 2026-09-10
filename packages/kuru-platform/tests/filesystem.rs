use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, PublicationPhase, make_executable,
    regular_file_info, require_private, seal_private,
};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::time::Duration;

fn fixture() -> (tempfile::TempDir, Directory) {
    let temporary = tempfile::tempdir().unwrap();
    let directory =
        Directory::ensure_private(&temporary.path().join("private space 日本語")).unwrap();
    (temporary, directory)
}

fn contents(mut file: File) -> Vec<u8> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn private_creation_reopen_and_sealing_preserve_content_and_identity() {
    let (_temporary, directory) = fixture();
    let name = OsStr::new("mémoire 日本語");
    let mut file = directory.create_new(name).unwrap();
    file.write_all(b"isolated memory").unwrap();
    let info = regular_file_info(&file).unwrap();
    assert_eq!(info.links, 1);
    assert_eq!(info.len, 15);
    require_private(&file).unwrap();
    directory.verify(name, &file).unwrap();
    seal_private(&file, false).unwrap();
    let reopened =
        Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned).unwrap();
    assert_eq!(reopened.identity(), directory.identity());
    assert_eq!(
        regular_file_info(&reopened.read(name).unwrap())
            .unwrap()
            .identity,
        info.identity
    );
    assert_eq!(contents(reopened.read(name).unwrap()), b"isolated memory");
    assert!(directory.create_new(name).is_err());
    assert_eq!(contents(reopened.read(name).unwrap()), b"isolated memory");
}

#[test]
fn newly_inherited_descendants_obey_the_private_parent_policy() {
    let (_temporary, directory) = fixture();
    let child = directory.path().join("inherited");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&child).unwrap();
    }
    #[cfg(windows)]
    fs::create_dir(&child).unwrap();
    let child = Directory::open(&child, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
    let mut file = child.create_new(OsStr::new("child-file")).unwrap();
    file.write_all(b"inherited private content").unwrap();
    require_private(&file).unwrap();
    assert_eq!(
        contents(child.read(OsStr::new("child-file")).unwrap()),
        b"inherited private content"
    );
}

#[test]
fn native_executable_access_does_not_change_file_identity() {
    let (_temporary, directory) = fixture();
    let mut executable = directory.create_new(OsStr::new("program")).unwrap();
    executable.write_all(b"fixture executable bytes").unwrap();
    let identity = regular_file_info(&executable).unwrap().identity;
    seal_private(&executable, true).unwrap();
    require_private(&executable).unwrap();
    make_executable(&executable).unwrap();
    assert_eq!(regular_file_info(&executable).unwrap().identity, identity);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            executable.metadata().unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}

#[test]
fn aliases_are_detected_by_handle_identity_and_rejected_by_checked_reads() {
    let (_temporary, directory) = fixture();
    let mut original = directory.create_new(OsStr::new("original")).unwrap();
    original.write_all(b"do not truncate").unwrap();
    fs::hard_link(
        directory.path().join("original"),
        directory.path().join("alias"),
    )
    .unwrap();
    let alias = File::open(directory.path().join("alias")).unwrap();
    assert_eq!(
        regular_file_info(&original).unwrap().identity,
        regular_file_info(&alias).unwrap().identity
    );
    assert_eq!(regular_file_info(&original).unwrap().links, 2);
    assert!(directory.read(OsStr::new("alias")).is_err());
    assert!(directory.read_write(OsStr::new("alias")).is_err());
    assert!(directory.lock_file(OsStr::new("alias")).is_err());
    assert!(seal_private(&original, false).is_err());
    assert_eq!(
        fs::read(directory.path().join("original")).unwrap(),
        b"do not truncate"
    );
}

#[test]
fn non_components_and_relative_directory_inputs_are_rejected() {
    let (_temporary, directory) = fixture();
    for name in [
        "",
        ".",
        "..",
        "parent/child",
        "child/.",
        "child/",
        "./child",
        "/absolute",
        "nul\0byte",
    ] {
        assert!(
            directory.create_new(OsStr::new(name)).is_err(),
            "accepted {name:?}"
        );
    }
    assert!(
        Directory::open(
            Path::new("relative"),
            Privacy::Inherited,
            NameRetention::Movable
        )
        .is_err()
    );
    assert!(
        Directory::open(
            &directory.path().join("../elsewhere"),
            Privacy::Inherited,
            NameRetention::Movable
        )
        .is_err()
    );
    assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
}

#[test]
fn checked_publication_preserves_occupied_targets_then_replaces_atomically() {
    let (_temporary, directory) = fixture();
    let mut old = directory.create_new(OsStr::new("published")).unwrap();
    old.write_all(b"old bytes").unwrap();
    let mut new = directory.create_new(OsStr::new("candidate")).unwrap();
    new.write_all(b"new bytes").unwrap();
    let identity = regular_file_info(&new).unwrap().identity;
    let error = directory
        .publish_file(
            &directory,
            OsStr::new("candidate"),
            &new,
            OsStr::new("published"),
            Publication::New,
        )
        .unwrap_err();
    assert_eq!(error.phase, PublicationPhase::Rejected);
    assert_eq!(error.error().kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read(directory.path().join("published")).unwrap(),
        b"old bytes"
    );
    directory
        .publish_file(
            &directory,
            OsStr::new("candidate"),
            &new,
            OsStr::new("published"),
            Publication::ReplaceRegular,
        )
        .unwrap();
    assert_eq!(
        regular_file_info(&directory.read(OsStr::new("published")).unwrap())
            .unwrap()
            .identity,
        identity
    );
    assert_eq!(
        fs::read(directory.path().join("published")).unwrap(),
        b"new bytes"
    );
    assert!(!directory.path().join("candidate").exists());
    old.rewind().unwrap();
    assert_eq!(contents(old), b"old bytes");
}

#[test]
fn publication_rejects_an_unrelated_source_and_non_file_target() {
    let (_temporary, directory) = fixture();
    let mut candidate = directory.create_new(OsStr::new("candidate")).unwrap();
    candidate.write_all(b"candidate bytes").unwrap();
    let other = directory.create_new(OsStr::new("other")).unwrap();
    let error = directory
        .publish_file(
            &directory,
            OsStr::new("candidate"),
            &other,
            OsStr::new("published"),
            Publication::New,
        )
        .unwrap_err();
    assert_eq!(error.phase, PublicationPhase::Rejected);
    assert!(!directory.path().join("published").exists());
    let _subdirectory =
        Directory::ensure_private(&directory.path().join("occupied-directory")).unwrap();
    assert!(
        directory
            .publish_file(
                &directory,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("occupied-directory"),
                Publication::ReplaceRegular
            )
            .is_err()
    );
    assert!(
        directory
            .publish_file(
                &directory,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("candidate"),
                Publication::ReplaceRegular
            )
            .is_err()
    );
    assert_eq!(
        fs::read(directory.path().join("candidate")).unwrap(),
        b"candidate bytes"
    );
    assert!(directory.path().join("occupied-directory").is_dir());
}

#[test]
fn new_publication_and_checked_writable_open_keep_existing_bytes_until_explicit_write() {
    let (_temporary, directory) = fixture();
    let mut candidate = directory.create_new(OsStr::new("candidate")).unwrap();
    candidate.write_all(b"first").unwrap();
    directory
        .publish_file(
            &directory,
            OsStr::new("candidate"),
            &candidate,
            OsStr::new("published"),
            Publication::New,
        )
        .unwrap();
    let mut writer = directory.read_write(OsStr::new("published")).unwrap();
    assert_eq!(contents(writer.try_clone().unwrap()), b"first");
    writer.write_all(b" second").unwrap();
    assert_eq!(
        contents(directory.read(OsStr::new("published")).unwrap()),
        b"first second"
    );
}

#[test]
fn directory_substitution_invalidates_previously_held_authority() {
    let (temporary, directory) = fixture();
    let mut file = directory.create_new(OsStr::new("record")).unwrap();
    file.write_all(b"original").unwrap();
    fs::rename(directory.path(), temporary.path().join("retired")).unwrap();
    let replacement = Directory::ensure_private(directory.path()).unwrap();
    replacement
        .create_new(OsStr::new("record"))
        .unwrap()
        .write_all(b"replacement")
        .unwrap();
    assert_ne!(replacement.identity(), directory.identity());
    assert!(directory.read(OsStr::new("record")).is_err());
    assert!(directory.verify(OsStr::new("record"), &file).is_err());
    assert_eq!(
        fs::read(temporary.path().join("retired/record")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(replacement.path().join("record")).unwrap(),
        b"replacement"
    );
}

#[test]
fn directory_moves_do_not_implicitly_convert_privacy_in_either_direction() {
    let (_temporary, parent) = fixture();
    for (index, source_policy, destination_policy) in [
        (0, Privacy::Inherited, Privacy::OwnerOnly),
        (1, Privacy::OwnerOnly, Privacy::Inherited),
    ] {
        let path = parent.path().join(format!("source-{index}"));
        let original = Directory::ensure_private(&path).unwrap();
        original
            .create_new(OsStr::new("record"))
            .unwrap()
            .write_all(b"retained source")
            .unwrap();
        let source = Directory::open(&path, source_policy, NameRetention::Movable).unwrap();
        let destination =
            Directory::open(parent.path(), destination_policy, NameRetention::Movable).unwrap();
        let name = format!("destination-{index}");
        let error = destination
            .move_new_directory(&source, OsStr::new(&name))
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert!(!parent.path().join(name).exists());
        assert_eq!(
            Directory::open(&path, source_policy, NameRetention::Movable)
                .unwrap()
                .identity(),
            source.identity()
        );
        assert_eq!(fs::read(path.join("record")).unwrap(), b"retained source");
    }
}

#[cfg(feature = "test-support")]
#[test]
fn real_process_lock_contention_survives_directory_publication() {
    let (_temporary, parent) = fixture();
    let source = Directory::ensure_private(&parent.path().join("stage")).unwrap();
    let lock = source.lock_file(OsStr::new("lifecycle.lock")).unwrap();
    lock.try_lock().unwrap();
    source.verify(OsStr::new("lifecycle.lock"), &lock).unwrap();
    assert_eq!(lock_attempt(source.path()), "busy\n");
    let occupied = Directory::ensure_private(&parent.path().join("occupied")).unwrap();
    let error = parent
        .move_new_directory(&source, OsStr::new("occupied"))
        .unwrap_err();
    assert_eq!(error.phase, PublicationPhase::Rejected);
    assert!(occupied.path().is_dir());
    let moved = parent
        .move_new_directory(&source, OsStr::new("active"))
        .unwrap();
    assert_eq!(moved.identity(), source.identity());
    moved.verify(OsStr::new("lifecycle.lock"), &lock).unwrap();
    assert_eq!(lock_attempt(moved.path()), "busy\n");
    drop(lock);
    assert_eq!(lock_attempt(moved.path()), "acquired\n");
}

#[cfg(all(unix, feature = "test-support"))]
fn lock_attempt(directory: &Path) -> String {
    use std::process::{Command, Stdio};
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-platform-fs-fixture"));
    command
        .arg(directory)
        .arg("lifecycle.lock")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();
    let output = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output.take(1024).read_to_end(&mut bytes).map(|_| bytes);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(Duration::from_secs(10));
    if result.is_err()
        || result
            .as_ref()
            .is_ok_and(|result| result.as_ref().is_ok_and(|bytes| bytes.len() == 1024))
    {
        let _ = child.kill();
    }
    let status = child.wait().unwrap();
    reader.join().unwrap();
    assert!(status.success(), "lock fixture failed: {status}");
    String::from_utf8(result.expect("lock fixture exceeded deadline").unwrap()).unwrap()
}

#[cfg(all(windows, feature = "test-support"))]
fn lock_attempt(directory: &Path) -> String {
    use kuru_platform::windows::process::{NativeSpawnSpec, Stdio};
    use tokio::io::AsyncReadExt;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut spec = NativeSpawnSpec::new(
                env!("CARGO_BIN_EXE_kuru-platform-fs-fixture").into(),
                directory.to_path_buf(),
            );
            spec.args = vec![directory.as_os_str().to_owned(), "lifecycle.lock".into()];
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
            }
            spec.stdout = Stdio::Pipe;
            let mut child = spec.spawn().await.unwrap();
            let mut output = child.take_stdout().unwrap().take(1024);
            let mut bytes = Vec::new();
            let read =
                tokio::time::timeout(Duration::from_secs(10), output.read_to_end(&mut bytes)).await;
            if read.is_err() || bytes.len() == 1024 {
                child.terminate().unwrap();
            }
            output
                .get_mut()
                .close(Duration::from_secs(10))
                .await
                .unwrap();
            let status = child.wait(Duration::from_secs(10)).await.unwrap();
            assert!(status.success(), "lock fixture failed: {status}");
            read.expect("lock fixture exceeded deadline").unwrap();
            String::from_utf8(bytes).unwrap().replace("\r\n", "\n")
        })
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn private_file_publication_rejects_an_ordinary_permissive_candidate_before_the_move() {
        let (_temporary, private) = fixture();
        let ordinary =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let mut original = private.create_new(OsStr::new("published")).unwrap();
        original.write_all(b"private original").unwrap();
        let mut candidate = ordinary.create_new(OsStr::new("candidate")).unwrap();
        candidate.write_all(b"ordinary candidate").unwrap();
        candidate
            .set_permissions(fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(require_private(&candidate).is_err());
        let error = private
            .publish_file(
                &ordinary,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("published"),
                Publication::ReplaceRegular,
            )
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert_eq!(
            fs::read(private.path().join("published")).unwrap(),
            b"private original"
        );
        assert_eq!(
            fs::read(private.path().join("candidate")).unwrap(),
            b"ordinary candidate"
        );
        assert_eq!(
            candidate.metadata().unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn native_punctuation_filenames_roundtrip_without_normalization() {
        let (_temporary, directory) = fixture();
        let names: Vec<_> = ["NUL", "a:b", "back\\slash", "trailing.", "space "]
            .into_iter()
            .map(OsString::from)
            .collect();
        for name in names {
            directory
                .create_new(&name)
                .unwrap()
                .write_all(name.as_encoded_bytes())
                .unwrap();
            assert_eq!(
                contents(directory.read(&name).unwrap()),
                name.as_encoded_bytes()
            );
        }
    }

    #[test]
    fn non_utf8_names_preserve_the_native_filesystems_behavior() {
        use std::os::unix::fs::OpenOptionsExt;
        let (_temporary, directory) = fixture();
        let native_name = OsString::from_vec(vec![b'n', 0xff, b'w']);
        let checked_name = OsString::from_vec(vec![b'c', 0xff, b'w']);
        let native = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.path().join(&native_name));
        match native {
            Ok(mut direct) => {
                direct.write_all(native_name.as_encoded_bytes()).unwrap();
                assert_eq!(
                    contents(directory.read(&native_name).unwrap()),
                    native_name.as_encoded_bytes()
                );
                directory
                    .create_new(&checked_name)
                    .unwrap()
                    .write_all(checked_name.as_encoded_bytes())
                    .unwrap();
                assert_eq!(
                    contents(directory.read(&checked_name).unwrap()),
                    checked_name.as_encoded_bytes()
                );
            }
            Err(native_error) => {
                // APFS rejects invalid UTF-8 natively. Preserve its exact error;
                // never create a lossy replacement filename to pretend success.
                let checked_error = directory.create_new(&checked_name).unwrap_err();
                assert_eq!(checked_error.raw_os_error(), native_error.raw_os_error());
                assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
            }
        }
    }

    #[test]
    fn unsafe_existing_permissions_are_rejected_without_repair() {
        let (_temporary, directory) = fixture();
        let file = directory.create_new(OsStr::new("record")).unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o640))
            .unwrap();
        assert!(require_private(&file).is_err());
        assert!(directory.read(OsStr::new("record")).is_err());
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o640);
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o770)).unwrap();
        assert!(Directory::ensure_private(directory.path()).is_err());
        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o770
        );
    }

    #[test]
    fn symlink_ancestors_leaves_and_special_files_cannot_read_or_modify_outside_data() {
        let (temporary, directory) = fixture();
        let outside = temporary.path().join("outside");
        fs::write(&outside, b"outside sentinel").unwrap();
        symlink(&outside, directory.path().join("leaf")).unwrap();
        assert!(directory.read(OsStr::new("leaf")).is_err());
        assert!(directory.read_write(OsStr::new("leaf")).is_err());
        assert!(directory.lock_file(OsStr::new("leaf")).is_err());
        symlink(directory.path(), temporary.path().join("ancestor")).unwrap();
        assert!(
            Directory::open(
                &temporary.path().join("ancestor"),
                Privacy::Inherited,
                NameRetention::Movable
            )
            .is_err()
        );
        assert!(Directory::ensure_private(&temporary.path().join("ancestor/new")).is_err());
        let fifo = directory.path().join("fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        assert!(directory.read(OsStr::new("fifo")).is_err());
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let socket = File::from(std::os::fd::OwnedFd::from(socket));
        assert!(regular_file_info(&socket).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"outside sentinel");
    }

    #[test]
    fn held_lock_name_substitution_is_detected_without_unlocking_the_original() {
        let (_temporary, directory) = fixture();
        let lock = directory.lock_file(OsStr::new("lock")).unwrap();
        lock.try_lock().unwrap();
        fs::rename(
            directory.path().join("lock"),
            directory.path().join("original-lock"),
        )
        .unwrap();
        let replacement = directory.lock_file(OsStr::new("lock")).unwrap();
        replacement.try_lock().unwrap();
        assert!(directory.verify(OsStr::new("lock"), &lock).is_err());
        let original = directory.lock_file(OsStr::new("original-lock")).unwrap();
        assert!(original.try_lock().is_err());
        directory.verify(OsStr::new("lock"), &replacement).unwrap();
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    #[test]
    fn native_device_stream_and_normalized_alias_names_are_rejected() {
        let (_temporary, directory) = fixture();
        for name in [
            "NUL",
            "aux.txt",
            "CON",
            "COM1",
            "LPT³.txt",
            "record:stream",
            "trailing.",
            "space ",
            "back\\slash",
        ] {
            assert!(
                directory.create_new(OsStr::new(name)).is_err(),
                "accepted {name:?}"
            );
        }
        assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
    }

    #[test]
    fn pinned_source_names_resist_replacement_until_their_handles_close() {
        let (_temporary, directory) = fixture();
        directory
            .create_new(OsStr::new("source"))
            .unwrap()
            .write_all(b"source bytes")
            .unwrap();
        let pinned =
            Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned).unwrap();
        let source = pinned.read(OsStr::new("source")).unwrap();
        let destination = directory.path().join("renamed");
        assert!(fs::rename(directory.path().join("source"), &destination).is_err());
        assert_eq!(
            fs::read(directory.path().join("source")).unwrap(),
            b"source bytes"
        );
        pinned.verify(OsStr::new("source"), &source).unwrap();
        drop(source);
        drop(pinned);
        fs::rename(directory.path().join("source"), &destination).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"source bytes");
    }
}
