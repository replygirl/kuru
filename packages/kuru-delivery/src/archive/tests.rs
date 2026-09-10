use super::*;
use crate::command::BlockingCommand as Command;
#[path = "../../tests/support/files.rs"]
mod files;
use files::symlink;

struct Fixture {
    _root: tempfile::TempDir,
    releases: PathBuf,
    destination: PathBuf,
    archive: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let releases = root.path().join("releases");
        let destination = root.path().join("bin");
        fs::create_dir_all(&releases).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("kuru"), "previous version").unwrap();
        let archive = releases.join(archive_name("0.1.0", TARGETS[0]).unwrap());
        Self {
            _root: root,
            releases,
            destination,
            archive,
        }
    }

    fn write(&self, entries: &[(&str, &[u8], u32, tar::EntryType)]) -> Vec<u8> {
        let mut archive = tar::Builder::new(
            GzBuilder::new()
                .mtime(0)
                .write(Vec::new(), Compression::default()),
        );
        for (name, bytes, mode, kind) in entries {
            let mut header = tar::Header::new_ustar();
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_entry_type(*kind);
            header.set_cksum();
            archive.append(&header, *bytes).unwrap();
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        fs::write(&self.archive, &bytes).unwrap();
        fs::write(
            self.releases.join("SHA256SUMS"),
            format!(
                "{}  {}\n",
                digest(&bytes),
                self.archive.file_name().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();
        bytes
    }

    fn valid(&self) -> Vec<u8> {
        self.write(&[
            (
                "kuru",
                b"#!/bin/sh\necho kuru 0.1.0\n",
                0o755,
                tar::EntryType::Regular,
            ),
            ("LICENSE", b"license", 0o644, tar::EntryType::Regular),
            ("README.md", b"readme", 0o644, tar::EntryType::Regular),
        ])
    }

    async fn install(&self) -> Result<PathBuf> {
        install(
            self.releases.to_str().unwrap(),
            "v0.1.0",
            &self.destination,
            Some(TARGETS[0]),
        )
        .await
    }

    fn unchanged(&self) {
        assert_eq!(
            fs::read_to_string(self.destination.join("kuru")).unwrap(),
            "previous version"
        );
        assert_eq!(
            fs::read_dir(&self.destination).unwrap().count(),
            1,
            "staging files leaked"
        );
    }
}

#[tokio::test]
async fn valid_release_replaces_executable_and_runs() {
    let fixture = Fixture::new();
    let current = fs::read(std::env::current_exe().unwrap()).unwrap();
    fixture.write(&[
        ("kuru", &current, 0o755, tar::EntryType::Regular),
        ("LICENSE", b"license", 0o644, tar::EntryType::Regular),
        ("README.md", b"readme", 0o644, tar::EntryType::Regular),
    ]);
    let path = fixture.install().await.unwrap();
    let result = Command::new(path).arg("--list").output().unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("valid_release_replaces_executable_and_runs")
    );
    assert_eq!(
        fs::read_dir(&fixture.destination).unwrap().count(),
        1 + usize::from(cfg!(windows))
    );
}

#[tokio::test]
async fn corrupted_archive_and_duplicate_checksums_preserve_existing_binary() {
    let fixture = Fixture::new();
    fixture.valid();
    fs::write(&fixture.archive, "corrupted").unwrap();
    assert!(
        fixture
            .install()
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    fixture.unchanged();
    fixture.valid();
    let manifest = fixture.releases.join("SHA256SUMS");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().repeat(2)).unwrap();
    assert!(
        fixture
            .install()
            .await
            .unwrap_err()
            .to_string()
            .contains("exactly once")
    );
    fixture.unchanged();
}

#[tokio::test]
async fn traversal_links_special_headers_nonexecutables_and_empty_files_are_rejected() {
    for (name, kind, mode, bytes) in [
        ("../kuru", tar::EntryType::Regular, 0o755, b"x".as_slice()),
        ("/tmp/kuru", tar::EntryType::Regular, 0o755, b"x"),
        ("kuru", tar::EntryType::Symlink, 0o755, b""),
        ("kuru", tar::EntryType::Link, 0o755, b""),
        ("kuru", tar::EntryType::Directory, 0o755, b""),
        ("pax", tar::EntryType::XHeader, 0o755, b"x"),
        ("kuru", tar::EntryType::Regular, 0o644, b"x"),
        ("kuru", tar::EntryType::Regular, 0o755, b""),
        ("LICENSE", tar::EntryType::Regular, 0o644, b"license"),
    ] {
        let fixture = Fixture::new();
        fixture.write(&[(name, bytes, mode, kind)]);
        assert!(
            fixture.install().await.is_err(),
            "accepted {name} {kind:?} {mode:o}"
        );
        fixture.unchanged();
    }
}

#[tokio::test]
async fn duplicate_entries_and_missing_binary_are_rejected() {
    let fixture = Fixture::new();
    let entry = ("kuru", b"x".as_slice(), 0o755, tar::EntryType::Regular);
    fixture.write(&[entry, entry]);
    assert!(
        fixture
            .install()
            .await
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    fixture.unchanged();
    fixture.write(&[]);
    assert!(
        fixture
            .install()
            .await
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    fixture.unchanged();
}

#[test]
fn gzip_expansion_is_bounded_before_tar_headers_are_parsed() {
    let fixture = Fixture::new();
    let bytes = fixture.write(&[("kuru", &vec![b'x'; 4096], 0o755, tar::EntryType::Regular)]);
    assert!(bytes.len() < 2048);
    assert!(
        install_archive(&bytes, &fixture.destination, 2048)
            .unwrap_err()
            .to_string()
            .contains("expanded release archive")
    );
    fixture.unchanged();
    assert!(install_archive(b"not gzip", &fixture.destination, MAX_ARCHIVE_BYTES).is_err());
    fixture.unchanged();
}

#[tokio::test]
async fn symlink_and_directory_destinations_are_not_replaced() {
    let fixture = Fixture::new();
    fixture.valid();
    let path = fixture.destination.join("kuru");
    let outside = fixture._root.path().join("outside");
    fs::write(&outside, "outside").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(
        fixture
            .install()
            .await
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(fixture.install().await.is_err());
    assert!(path.is_dir());
}

#[tokio::test]
async fn transport_rejects_unsafe_sources_and_bounds_local_assets() {
    for base in [
        "http://example.invalid",
        "https://u:p@example.invalid",
        "ftp://example.invalid",
        "https://example.invalid/#fragment",
        "https://example.invalid/?query",
        "file:///tmp",
    ] {
        assert!(read_asset(base, "SHA256SUMS", 10).await.is_err(), "{base}");
    }
    let fixture = Fixture::new();
    fs::write(fixture.releases.join("large"), [b'x'; 11]).unwrap();
    assert!(
        read_asset(fixture.releases.to_str().unwrap(), "large", 10)
            .await
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    assert!(
        read_asset(fixture.releases.to_str().unwrap(), "missing", 10)
            .await
            .is_err()
    );
}

#[test]
fn versions_platforms_and_manifests_are_validated_before_installation() {
    for version in [
        "latest",
        "../../x",
        "1.0",
        "1.0.0;id",
        "v1.0.0/evil",
        "1.0.0-",
        "1..0",
    ] {
        assert!(checked_version(version).is_err(), "{version}");
    }
    assert_eq!(checked_version("v1.2.3-rc.1").unwrap(), "1.2.3-rc.1");
    assert!(archive_name("0.1.0", "unknown").is_err());
    assert!(TARGETS.contains(&host_target().unwrap()));
    assert!(expected_digest(&[0xff], "kuru").is_err());
    for manifest in ["invalid kuru", "x  kuru", "", "000  other"] {
        assert!(expected_digest(manifest.as_bytes(), "kuru").is_err());
    }
    assert_eq!(
        expected_digest(format!("{} *kuru", "AB".repeat(32)).as_bytes(), "kuru").unwrap(),
        "ab".repeat(32)
    );
}

#[tokio::test]
async fn packager_roundtrip_is_reproducible_and_includes_documentation() {
    let fixture = Fixture::new();
    let binary = fixture._root.path().join("fixture-kuru");
    files::executable(
        &binary,
        &fs::read(std::env::current_exe().unwrap()).unwrap(),
    );
    let archive = package(&binary, TARGETS[0], "0.2.0", &fixture.releases).unwrap();
    let repeated = package(
        &binary,
        TARGETS[0],
        "0.2.0",
        &fixture._root.path().join("repeat"),
    )
    .unwrap();
    assert_eq!(fs::read(&archive).unwrap(), fs::read(repeated).unwrap());
    fs::copy(
        fixture.releases.join(format!(
            "{}.sha256",
            archive.file_name().unwrap().to_str().unwrap()
        )),
        fixture.releases.join("SHA256SUMS"),
    )
    .unwrap();
    let installed = install(
        fixture.releases.to_str().unwrap(),
        "0.2.0",
        &fixture.destination,
        Some(TARGETS[0]),
    )
    .await
    .unwrap();
    let result = Command::new(installed).arg("--list").output().unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("packager_roundtrip_is_reproducible_and_includes_documentation")
    );
    let data = fs::read(archive).unwrap();
    let mut archive = tar::Archive::new(GzDecoder::new(data.as_slice()));
    let names: Vec<_> = archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            PathBuf::from("kuru"),
            PathBuf::from("LICENSE"),
            PathBuf::from("README.md")
        ]
    );
}

#[test]
fn packaging_requires_a_regular_executable_and_known_target() {
    let fixture = Fixture::new();
    let path = fixture.destination.join("kuru");
    #[cfg(unix)]
    assert!(package(&path, TARGETS[0], "0.1.0", &fixture.releases).is_err());
    #[cfg(windows)]
    {
        let directory = fixture.destination.join("not-an-executable");
        fs::create_dir(&directory).unwrap();
        assert!(package(&directory, TARGETS[4], "0.1.0", &fixture.releases).is_err());
    }
    make_executable(&File::open(&path).unwrap()).unwrap();
    assert!(package(&path, "unknown", "0.1.0", &fixture.releases).is_err());
    let link = fixture.destination.join("link");
    symlink(path, &link).unwrap();
    assert!(package(&link, TARGETS[0], "0.1.0", &fixture.releases).is_err());
    assert!(
        package(
            Path::new("/missing/kuru"),
            TARGETS[0],
            "0.1.0",
            &fixture.releases
        )
        .is_err()
    );
}

#[test]
fn packaging_rejects_hardlinked_outputs_without_modifying_the_input() {
    for checksum in [false, true] {
        let fixture = Fixture::new();
        let binary = fixture._root.path().join("input");
        fs::write(&binary, b"#!/bin/sh\necho input\n").unwrap();
        make_executable(&File::open(&binary).unwrap()).unwrap();
        let name = archive_name("0.1.0", TARGETS[0]).unwrap();
        let output = fixture.releases.join(if checksum {
            format!("{name}.sha256")
        } else {
            name
        });
        fs::hard_link(&binary, &output).unwrap();
        let error = package(&binary, TARGETS[0], "0.1.0", &fixture.releases).unwrap_err();
        assert!(format!("{error:#}").contains("link"), "{error:#}");
        assert_eq!(fs::read(&binary).unwrap(), b"#!/bin/sh\necho input\n");
        assert_eq!(fs::read(output).unwrap(), fs::read(binary).unwrap());
        assert_eq!(fs::read_dir(&fixture.releases).unwrap().count(), 1);
    }
}

#[test]
fn packaging_failure_preserves_previous_artifacts_and_cleans_staging() {
    let fixture = Fixture::new();
    let binary = fixture._root.path().join("input");
    fs::write(&binary, b"#!/bin/sh\necho input\n").unwrap();
    make_executable(&File::open(&binary).unwrap()).unwrap();
    let archive = package(&binary, TARGETS[0], "0.1.0", &fixture.releases).unwrap();
    let checksum = fixture.releases.join(format!(
        "{}.sha256",
        archive.file_name().unwrap().to_str().unwrap()
    ));
    let previous_archive = fs::read(&archive).unwrap();
    let previous_checksum = fs::read(&checksum).unwrap();
    // A sparse file exercises the real size guard without allocating its body.
    File::options()
        .write(true)
        .open(&binary)
        .unwrap()
        .set_len(MAX_ARCHIVE_BYTES as u64 + 1)
        .unwrap();
    let error = package(&binary, TARGETS[0], "0.1.0", &fixture.releases).unwrap_err();
    assert!(error.to_string().contains("size limit"), "{error}");
    assert_eq!(fs::read(&archive).unwrap(), previous_archive);
    assert_eq!(fs::read(&checksum).unwrap(), previous_checksum);
    assert_eq!(fs::read_dir(&fixture.releases).unwrap().count(), 2);
}

#[test]
fn packaging_does_not_follow_archive_or_checksum_output_symlinks() {
    for checksum in [false, true] {
        let fixture = Fixture::new();
        let binary = fixture._root.path().join("input");
        fs::write(&binary, b"#!/bin/sh\necho input\n").unwrap();
        make_executable(&File::open(&binary).unwrap()).unwrap();
        let outside = fixture._root.path().join("outside");
        fs::write(&outside, b"outside preserved").unwrap();
        let name = archive_name("0.1.0", TARGETS[0]).unwrap();
        let output = fixture.releases.join(if checksum {
            format!("{name}.sha256")
        } else {
            name
        });
        symlink(&outside, &output).unwrap();
        let error = package(&binary, TARGETS[0], "0.1.0", &fixture.releases).unwrap_err();
        assert!(error.to_string().contains("not a symlink"), "{error}");
        assert!(output.is_symlink());
        assert_eq!(fs::read(outside).unwrap(), b"outside preserved");
        assert_eq!(fs::read_dir(&fixture.releases).unwrap().count(), 1);
    }
}
