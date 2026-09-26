#![cfg(feature = "tooling")]

use kuru_archive::zip::{self, Archive, Limits, MemberKind, MemberSpec, WriteMember};
use kuru_delivery::{
    archive::{self, MAX_ARCHIVE_BYTES},
    command::BlockingCommand as Command,
    shell_support,
    targets::{self, ArchiveFormat},
};
use std::{fs, path::Path};

#[path = "support/files.rs"]
mod files;

const WINDOWS: &str = "x86_64-pc-windows-msvc";

fn limits() -> Limits {
    Limits {
        max_compressed_bytes: MAX_ARCHIVE_BYTES as u64,
        max_expanded_bytes: MAX_ARCHIVE_BYTES as u64,
        allow_ntfs_timestamps: false,
    }
}

fn manifest(directory: &Path, bytes: &[u8]) {
    let name = archive::archive_name("0.2.0", WINDOWS).unwrap();
    fs::write(directory.join(&name), bytes).unwrap();
    fs::write(
        directory.join("SHA256SUMS"),
        format!("{}  {name}\n", archive::digest(bytes)),
    )
    .unwrap();
}

#[tokio::test]
async fn real_packager_writes_exact_reproducible_windows_zip_and_installer_runs_host_fixture() {
    let root = tempfile::tempdir().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    let releases = root.path().join("release files");
    let path = archive::package(binary, WINDOWS, "0.2.0", &releases).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(path.ends_with("kuru-0.2.0-x86_64-pc-windows-msvc.zip"));
    let expected = ["kuru.exe", "LICENSE", "README.md"].map(|name| MemberSpec {
        name,
        kind: MemberKind::File,
        max_bytes: MAX_ARCHIVE_BYTES as u64,
        exact_bytes: None,
        unix_mode: Some(if name == "kuru.exe" {
            0o100755
        } else {
            0o100644
        }),
    });
    let mut zip = Archive::open(&bytes, &expected, limits()).unwrap();
    let mut payload = Vec::new();
    zip.copy("kuru.exe", &mut payload).unwrap();
    assert_eq!(payload, fs::read(binary).unwrap());
    for (name, expected) in [
        ("LICENSE", include_bytes!("../../../LICENSE").as_slice()),
        ("README.md", include_bytes!("../../../README.md").as_slice()),
    ] {
        let mut payload = Vec::new();
        zip.copy(name, &mut payload).unwrap();
        assert_eq!(payload, expected);
    }
    drop(zip);
    archive::package(binary, WINDOWS, "0.2.0", &releases).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let generated = root.path().join("generated support");
    for name in shell_support::NAMES {
        let file = generated.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("generated {name} for {WINDOWS}\n")).unwrap();
    }
    let support = shell_support::package(&generated, WINDOWS, "0.2.0", &releases).unwrap();
    fs::write(
        releases.join("SHA256SUMS"),
        format!(
            "{}{}",
            fs::read_to_string(path.with_extension("zip.sha256")).unwrap(),
            fs::read_to_string(support.with_extension("zip.sha256")).unwrap()
        ),
    )
    .unwrap();
    let destination = root.path().join("installed Unicode λ");
    let installed = archive::install(
        releases.to_str().unwrap(),
        "0.2.0",
        &destination,
        Some(WINDOWS),
    )
    .await
    .unwrap();
    assert_eq!(installed.file_name().unwrap(), "kuru.exe");
    assert_eq!(fs::read(&installed).unwrap(), fs::read(binary).unwrap());
    let result = Command::new(installed).arg("--version").output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"native fixture 0.2.0\n");
    let installed_support = destination.join("share/kuru/0.2.0").join(WINDOWS);
    assert_eq!(
        shell_support::read_generated(&installed_support).unwrap(),
        shell_support::read_generated(&generated).unwrap()
    );
    assert_eq!(
        fs::read(destination.join("share/man/man1/kuru.1")).unwrap(),
        fs::read(generated.join("man/kuru.1")).unwrap()
    );
    assert_eq!(
        fs::read_dir(destination).unwrap().count(),
        2 + usize::from(cfg!(windows))
    );
}

#[tokio::test]
async fn verified_zip_wrong_inventory_modes_and_crc_preserve_previous_image() {
    let root = tempfile::tempdir().unwrap();
    let releases = root.path().join("releases");
    let destination = root.path().join("bin");
    fs::create_dir(&releases).unwrap();
    fs::create_dir(&destination).unwrap();
    let installed = destination.join("kuru.exe");
    fs::write(&installed, b"previous").unwrap();
    let identity = files::identity(&installed);
    let regular = |name, executable| WriteMember {
        name,
        kind: MemberKind::File,
        bytes: b"candidate must never run",
        executable,
    };
    let valid = || {
        vec![
            regular("kuru.exe", true),
            regular("LICENSE", false),
            regular("README.md", false),
        ]
    };
    let mut missing = valid();
    missing.pop();
    let mut wrong_mode = valid();
    wrong_mode[0].executable = false;
    let mut wrong_name = valid();
    wrong_name[0].name = "kuru";
    let mut extra = valid();
    extra.push(regular("extra", false));
    let mut corrupt_crc = zip::write(&valid(), limits()).unwrap();
    // Both records agree with each other but deliberately disagree with decoded
    // content. A physical-only ZIP inventory check is insufficient.
    corrupt_crc[14] ^= 1;
    let central = corrupt_crc
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    corrupt_crc[central + 16] ^= 1;
    let malformed = [
        zip::write(&missing, limits()).unwrap(),
        zip::write(&wrong_mode, limits()).unwrap(),
        zip::write(&wrong_name, limits()).unwrap(),
        zip::write(&extra, limits()).unwrap(),
        corrupt_crc,
    ];
    for bytes in malformed {
        manifest(&releases, &bytes);
        assert!(
            archive::install(
                releases.to_str().unwrap(),
                "0.2.0",
                &destination,
                Some(WINDOWS)
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read(&installed).unwrap(), b"previous");
        assert_eq!(files::identity(&installed), identity);
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    }
}

#[test]
fn all_catalog_assets_share_release_naming_and_keep_ustar_contracts() {
    let root = tempfile::tempdir().unwrap();
    // Cargo's top-level binary aliases have multiple links on Linux. Make that
    // source shape explicit on every host rather than relying on Cargo's choice.
    let binary = root.path().join("compiled fixture");
    fs::copy(env!("CARGO_BIN_EXE_kuru-delivery-fixture"), &binary).unwrap();
    let alias = root.path().join("cargo alias");
    fs::hard_link(&binary, &alias).unwrap();
    let identity = files::identity(&binary);
    let expected_payload = fs::read(&binary).unwrap();
    let checked = kuru_platform::fs::Directory::open(
        root.path(),
        kuru_platform::fs::Privacy::Inherited,
        kuru_platform::fs::NameRetention::Movable,
    )
    .unwrap();
    assert!(checked.read(alias.file_name().unwrap()).is_err());
    for target in &targets::CATALOG {
        let path = archive::package(&alias, target.triple, "0.2.0", root.path()).unwrap();
        assert!(path.ends_with(archive::archive_name("0.2.0", target.triple).unwrap()));
        if target.format == ArchiveFormat::TarGz {
            let input = fs::read(path).unwrap();
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(input.as_slice()));
            let entries: Vec<_> = tar
                .entries()
                .unwrap()
                .map(|entry| {
                    let mut entry = entry.unwrap();
                    assert!(entry.header().entry_type().is_file());
                    if entry.path_bytes().as_ref() == b"kuru" {
                        let mut payload = Vec::new();
                        std::io::Read::read_to_end(&mut entry, &mut payload).unwrap();
                        assert_eq!(payload, expected_payload);
                    }
                    (entry.path_bytes().to_vec(), entry.header().mode().unwrap())
                })
                .collect();
            assert_eq!(
                entries,
                vec![
                    (b"kuru".to_vec(), 0o755),
                    (b"LICENSE".to_vec(), 0o644),
                    (b"README.md".to_vec(), 0o644)
                ]
            );
        } else {
            let bytes = fs::read(path).unwrap();
            let expected = ["kuru.exe", "LICENSE", "README.md"].map(|name| MemberSpec {
                name,
                kind: MemberKind::File,
                max_bytes: MAX_ARCHIVE_BYTES as u64,
                exact_bytes: None,
                unix_mode: Some(if name == "kuru.exe" {
                    0o100755
                } else {
                    0o100644
                }),
            });
            let mut archive = Archive::open(&bytes, &expected, limits()).unwrap();
            let mut payload = Vec::new();
            archive.copy("kuru.exe", &mut payload).unwrap();
            assert_eq!(payload, expected_payload);
        }
        assert_eq!(files::identity(&binary), identity);
        assert_eq!(files::identity(&alias), identity);
        assert_eq!(fs::read(&binary).unwrap(), expected_payload);
    }
    assert_eq!(
        kuru_platform::fs::regular_file_info(&fs::File::open(&binary).unwrap())
            .unwrap()
            .links,
        2
    );
}
