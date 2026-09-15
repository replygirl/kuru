use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, symlink};

const SCRIPT: &[u8] = b"#!/bin/sh\nprintf 'dolt version 2.3.3\\n'\n";
const LICENSE: &[u8] = b"Fixture license and dependency notices\n";
type ArchiveEntries = Vec<(String, tar::EntryType, u32, Vec<u8>)>;
static VALID_FIXTURE: std::sync::LazyLock<Fixture> =
    std::sync::LazyLock::new(|| Fixture::new(|_| {}));

struct Fixture {
    bytes: Vec<u8>,
    expanded: u64,
    archive_digest: String,
    binary_digest: String,
    binary_bytes: u64,
    license_digest: String,
}

impl Fixture {
    fn new(mut edit: impl FnMut(&mut ArchiveEntries)) -> Self {
        let mut entries = vec![
            ("fixture/".into(), tar::EntryType::Directory, 0o755, vec![]),
            (
                "fixture/bin/".into(),
                tar::EntryType::Directory,
                0o755,
                vec![],
            ),
            (
                "fixture/bin/dolt".into(),
                tar::EntryType::Regular,
                0o755,
                SCRIPT.to_vec(),
            ),
            (
                "fixture/LICENSES".into(),
                tar::EntryType::Regular,
                0o644,
                LICENSE.to_vec(),
            ),
        ];
        edit(&mut entries);
        let mut archive = tar::Builder::new(Vec::new());
        for (path, kind, mode, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_entry_type(kind);
            header.set_mode(mode);
            header.set_size(content.len() as u64);
            if kind.is_symlink() || kind.is_hard_link() {
                header.set_link_name("/tmp/unowned-dolt").unwrap();
            }
            header.set_cksum();
            archive.append(&header, content.as_slice()).unwrap();
        }
        Self::from_expanded(archive.into_inner().unwrap())
    }

    fn from_expanded(expanded: Vec<u8>) -> Self {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&expanded).unwrap();
        let bytes = encoder.finish().unwrap();
        Self {
            expanded: expanded.len() as u64,
            archive_digest: digest(&bytes),
            binary_digest: digest(SCRIPT),
            binary_bytes: SCRIPT.len() as u64,
            license_digest: digest(LICENSE),
            bytes,
        }
    }

    fn spec(&self) -> Asset<'_> {
        Asset {
            target: "fixture-target",
            stem: "fixture",
            format: "tar.gz",
            executable_name: "dolt",
            compressed_bytes: self.bytes.len() as u64,
            archive_sha256: &self.archive_digest,
            expanded_bytes: self.expanded,
            executable_bytes: self.binary_bytes,
            executable_sha256: &self.binary_digest,
            license_bytes: LICENSE.len() as u64,
            license_sha256: &self.license_digest,
        }
    }

    fn extract(&self, directory: &Path) -> Result<PathBuf> {
        let candidate = directory.join("candidate");
        extract(&self.bytes, &candidate, self.spec())?;
        Ok(candidate)
    }
}

fn digest(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

fn executable(path: &Path, content: &[u8]) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn installs_only_fixed_payloads_and_preserves_notices_and_previous_install() {
    let temporary = crate::test_support::tempdir().unwrap();
    let fixture = Fixture::new(|_| {});
    let candidate = fixture.extract(temporary.path()).unwrap();
    assert_eq!(fs::read(candidate.join("dolt")).unwrap(), SCRIPT);
    assert_eq!(fs::read(candidate.join("LICENSES")).unwrap(), LICENSE);
    assert_eq!(fs::read_dir(&candidate).unwrap().count(), 2);
    assert_eq!(
        fs::metadata(candidate.join("dolt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
    assert_eq!(
        fs::metadata(candidate.join("LICENSES"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    let destination = temporary.path().join("installed");
    activate(&candidate, &destination).unwrap();
    let binary = verified_cache(&destination, fixture.spec()).await.unwrap();
    assert_eq!(binary, destination.join("dolt"));
    assert!(activate(&candidate, &destination).is_err());
    assert_eq!(fs::read(binary).unwrap(), SCRIPT);
}

#[test]
fn rejects_archive_links_duplicates_missing_entries_metadata_and_bad_payloads() {
    type Mutation = Box<dyn FnMut(&mut ArchiveEntries)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|entries| entries.push(entries[2].clone())),
        Box::new(|entries| {
            entries.remove(0);
        }),
        Box::new(|entries| entries[2].1 = tar::EntryType::Symlink),
        Box::new(|entries| entries[2].1 = tar::EntryType::Link),
        Box::new(|entries| entries[2].1 = tar::EntryType::GNULongName),
        Box::new(|entries| entries[2].1 = tar::EntryType::XHeader),
        Box::new(|entries| entries[2].0 = "outside/dolt".into()),
        Box::new(|entries| entries[0].1 = tar::EntryType::Regular),
        Box::new(|entries| entries[0].2 = 0o777),
        Box::new(|entries| entries[2].2 = 0o644),
        Box::new(|entries| entries[2].3.push(b'!')),
        Box::new(|entries| entries[2].3[0] = b'!'),
        Box::new(|entries| entries[3].3[0] = b'!'),
        Box::new(|entries| {
            entries.push((
                "fixture/extra".into(),
                tar::EntryType::Regular,
                0o644,
                vec![],
            ))
        }),
    ];
    for mutation in mutations {
        let temporary = tempfile::tempdir().unwrap();
        assert!(Fixture::new(mutation).extract(temporary.path()).is_err());
    }
}

#[test]
fn rejects_compressed_corruption_truncation_expansion_and_trailing_data() {
    let original = Fixture::new(|_| {});
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("candidate");
    let mut spec = original.spec();
    spec.archive_sha256 = "incorrect";
    assert!(extract(&original.bytes, &destination, spec).is_err());
    spec = original.spec();
    spec.expanded_bytes -= 1;
    assert!(extract(&original.bytes, &destination, spec).is_err());
    spec.expanded_bytes = MAX_EXPANDED + 1;
    assert!(extract(&original.bytes, &destination, spec).is_err());
    spec = original.spec();
    spec.compressed_bytes += 1;
    assert!(extract(&original.bytes, &destination, spec).is_err());
    spec.compressed_bytes = MAX_COMPRESSED + 1;
    assert!(extract(&original.bytes, &destination, spec).is_err());
    let mut expanded = Vec::new();
    GzDecoder::new(original.bytes.as_slice())
        .read_to_end(&mut expanded)
        .unwrap();
    expanded.push(b'!');
    let trailing = Fixture::from_expanded(expanded);
    let other = tempfile::tempdir().unwrap();
    assert!(trailing.extract(other.path()).is_err());
    let mut trailing = Fixture::new(|_| {});
    trailing.bytes.extend_from_slice(b"unexpected gzip tail");
    trailing.archive_digest = digest(&trailing.bytes);
    let other = tempfile::tempdir().unwrap();
    assert!(trailing.extract(other.path()).is_err());
    let mut corrupt = Fixture::new(|_| {});
    corrupt.bytes[10] ^= 0xff;
    corrupt.archive_digest = digest(&corrupt.bytes);
    let other = tempfile::tempdir().unwrap();
    assert!(corrupt.extract(other.path()).is_err());
}

#[tokio::test]
async fn corrupt_cache_is_rejected_before_execution_and_links_are_never_adopted() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(|_| {});
    let candidate = fixture.extract(temporary.path()).unwrap();
    fs::set_permissions(candidate.join("dolt"), fs::Permissions::from_mode(0o700)).unwrap();
    let marker = temporary.path().join("must-not-exist");
    let mut corrupted = SCRIPT.to_vec();
    corrupted[0] = b'!';
    executable(&candidate.join("dolt"), &corrupted);
    assert!(
        verified_cache(&candidate, fixture.spec())
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum mismatch")
    );
    executable(
        &candidate.join("dolt"),
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()).as_bytes(),
    );
    assert!(verified_cache(&candidate, fixture.spec()).await.is_err());
    assert!(!marker.exists());
    fs::remove_file(candidate.join("dolt")).unwrap();
    symlink("/bin/sh", candidate.join("dolt")).unwrap();
    assert!(verified_cache(&candidate, fixture.spec()).await.is_err());
    let link = temporary.path().join("linked");
    symlink(&candidate, &link).unwrap();
    assert!(private_directory(&link).is_err());
    let link = temporary.path().join("dangling");
    symlink(temporary.path().join("absent"), &link).unwrap();
    assert!(private_directory(&link).is_err());
    let hard_link = temporary.path().join("hard-link");
    fs::hard_link(candidate.join("LICENSES"), &hard_link).unwrap();
    assert!(checked_regular(&hard_link, false).is_err());
    let shared = temporary.path().join("shared");
    fs::create_dir(&shared).unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_directory(&shared).is_err());
}

#[test]
fn cold_probe_copy_rejects_corrupt_source_and_corrupt_copied_bytes() {
    let fixture = &*VALID_FIXTURE;

    let source_root = crate::test_support::tempdir().unwrap();
    let source_candidate = fixture.extract(source_root.path()).unwrap();
    let mut corrupt_source = SCRIPT.to_vec();
    corrupt_source[0] = b'!';
    fs::set_permissions(
        source_candidate.join("dolt"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    executable(&source_candidate.join("dolt"), &corrupt_source);
    let source_error = prepare_cold_probe(
        &source_candidate,
        &source_root.path().join("probe"),
        fixture.spec(),
    )
    .unwrap_err();
    assert!(
        format!("{source_error:#}").contains("payload checksum mismatch"),
        "{source_error:#}"
    );

    let copy_root = crate::test_support::tempdir().unwrap();
    let copy_candidate = fixture.extract(copy_root.path()).unwrap();
    let copy_error = prepare_cold_probe_observed(
        &copy_candidate,
        &copy_root.path().join("probe"),
        fixture.spec(),
        |copy| {
            std::io::Seek::rewind(copy)?;
            copy.write_all(b"!")?;
            Ok(())
        },
    )
    .unwrap_err();
    assert!(
        format!("{copy_error:#}").contains("payload checksum mismatch"),
        "{copy_error:#}"
    );
}

#[tokio::test]
async fn failing_exact_version_probe_never_activates_and_releases_installation_authority() {
    const FAILING: &[u8] = b"#!/bin/sh\nexit 17\n";
    static FIXTURE: std::sync::LazyLock<Fixture> = std::sync::LazyLock::new(|| {
        let mut fixture = Fixture::new(|entries| entries[2].3 = FAILING.to_vec());
        fixture.binary_digest = digest(FAILING);
        fixture.binary_bytes = FAILING.len() as u64;
        fixture
    });
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    let (staged, received) = tokio::sync::oneshot::channel();
    let error = provision_with_extractor(
        &MemoryConfig::default(),
        &cache,
        FIXTURE.spec(),
        Cow::Borrowed(&FIXTURE.bytes),
        move |bytes, candidate, asset| {
            extract(bytes, candidate, asset)?;
            staged.send(candidate.to_owned()).unwrap();
            Ok(())
        },
    )
    .await
    .unwrap_err();
    let candidate = received.await.unwrap();
    assert!(format!("{error:#}").contains("Dolt version probe failed"));
    assert!(!candidate.exists());
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
    drop(lock);
}

#[tokio::test]
async fn stable_lock_waits_times_out_and_does_not_delete_a_held_inode() {
    let temporary = crate::test_support::tempdir().unwrap();
    let first = cache_lock(temporary.path(), Duration::from_secs(1))
        .await
        .unwrap();
    let inode = fs::metadata(temporary.path().join(".install.lock"))
        .unwrap()
        .ino();
    assert!(
        cache_lock(temporary.path(), Duration::from_millis(30))
            .await
            .is_err()
    );
    assert_eq!(
        fs::metadata(temporary.path().join(".install.lock"))
            .unwrap()
            .ino(),
        inode
    );
    drop(first);
    let second = cache_lock(temporary.path(), Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(second.metadata().unwrap().ino(), inode);
    drop(second);
    let locked = cache_lock(temporary.path(), Duration::from_secs(1))
        .await
        .unwrap();
    let waiter = cache_lock(temporary.path(), Duration::from_secs(2));
    tokio::pin!(waiter);
    assert!(futures::poll!(&mut waiter).is_pending());
    fs::remove_file(temporary.path().join(".install.lock")).unwrap();
    fs::write(temporary.path().join(".install.lock"), b"replacement").unwrap();
    drop(locked);
    assert!(waiter.await.is_err());
    let unrelated = temporary.path().join("unrelated");
    fs::write(&unrelated, b"leave this alone").unwrap();
    fs::remove_file(temporary.path().join(".install.lock")).unwrap();
    symlink(&unrelated, temporary.path().join(".install.lock")).unwrap();
    assert!(
        cache_lock(temporary.path(), Duration::from_secs(1))
            .await
            .is_err()
    );
    assert_eq!(fs::read(unrelated).unwrap(), b"leave this alone");
}

#[tokio::test]
async fn private_version_probe_requires_exact_version_and_bounds_process_and_output() {
    let temporary = tempfile::tempdir().unwrap();
    let binary = temporary.path().join("dolt");
    executable(&binary, SCRIPT);
    verify_version(&binary, &temporary.path().join("valid"))
        .await
        .unwrap();
    executable(&binary, b"#!/bin/sh\nprintf 'dolt version 1.0.0\\n'\n");
    assert!(
        verify_version(&binary, &temporary.path().join("old"))
            .await
            .unwrap_err()
            .to_string()
            .contains("requires full Dolt")
    );
    executable(&binary, b"#!/bin/sh\nexit 3\n");
    assert!(
        verify_version(&binary, &temporary.path().join("error"))
            .await
            .is_err()
    );
    executable(&binary, b"#!/bin/sh\nexec /bin/sleep 10\n");
    assert!(
        verify_version_with_timeout(
            &binary,
            &temporary.path().join("slow"),
            Duration::from_millis(40)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out")
    );
    executable(
        &binary,
        b"#!/bin/sh\nwhile :; do printf 'too much output too much output too much output'; done\n",
    );
    assert!(
        verify_version(&binary, &temporary.path().join("large"))
            .await
            .unwrap_err()
            .to_string()
            .contains("output exceeded")
    );
    executable(&binary, b"#!/bin/sh\nprintf '\\377'\n");
    assert!(
        verify_version(&binary, &temporary.path().join("utf8"))
            .await
            .is_err()
    );
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        verify_version(&binary, &temporary.path().join("mode"))
            .await
            .is_err()
    );
    executable(&binary, b"not an executable file");
    assert!(
        verify_version(&binary, &temporary.path().join("format"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn isolated_probe_sees_only_private_settings_and_preserves_private_server_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let binary = temporary.path().join("dolt");
    executable(&binary, b"#!/bin/sh\n[ \"$DOLT_DISABLE_EVENT_FLUSH\" = 1 ] || exit 11\n[ \"$HOME\" = \"$PWD/home\" ] || exit 12\n[ \"$DOLT_ROOT_PATH\" = \"$PWD/root\" ] || exit 13\n[ \"$TMPDIR\" = \"$PWD/tmp\" ] || exit 14\n[ -z \"$DOLT_ROOT_PASSWORD$DOLT_CLI_PASSWORD$ANTHROPIC_API_KEY$OPENAI_API_KEY\" ] || exit 15\n/bin/cat \"$DOLT_ROOT_PATH/.dolt/config_global.json\" | /usr/bin/grep -q '\"versioncheck.disabled\":\"true\"' || exit 16\nprintf 'dolt version 2.3.3\\n'\n");
    let home = temporary.path().join("private café home");
    prepare_private_home(&home).unwrap();
    let home = home.canonicalize().unwrap();
    let config = home.join("root/.dolt/config_global.json");
    fs::write(
        &config,
        br#"{"sqlserver.global.server_uuid":"preserve","versioncheck.disabled":"false"}"#,
    )
    .unwrap();
    // Dolt itself replaces its config with ordinary Unix permissions between
    // starts. The private parent still confines it, and the next preparation
    // must preserve identity settings while restoring a private new record.
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    verify_version(&binary, &home).await.unwrap();
    let actual: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(actual["sqlserver.global.server_uuid"], "preserve");
    assert_eq!(actual["metrics.disabled"], "true");
    assert_eq!(actual["versioncheck.disabled"], "true");
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&config, b"invalid").unwrap();
    assert!(prepare_private_home(&home).is_err());
    fs::write(&config, vec![b' '; 65 * 1024]).unwrap();
    assert!(prepare_private_home(&home).is_err());
    fs::remove_file(&config).unwrap();
    symlink("/tmp/missing-config", &config).unwrap();
    assert!(prepare_private_home(&home).is_err());
    fs::remove_file(&config).unwrap();
    assert_eq!(
        fs::read_dir(home.join("root/.dolt/staging"))
            .unwrap()
            .count(),
        0
    );
    fs::remove_dir(home.join("root/.dolt/staging")).unwrap();
    fs::remove_dir(home.join("root/.dolt")).unwrap();
    symlink(temporary.path(), home.join("root/.dolt")).unwrap();
    assert!(prepare_private_home(&home).is_err());
}

#[tokio::test]
async fn explicit_binary_wins_and_invalid_configured_cache_is_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("unused-cache");
    let binary = temporary.path().join("explicit-dolt");
    executable(&binary, SCRIPT);
    let mut config = MemoryConfig {
        offline: true,
        dolt_binary: Some(binary.clone()),
        ..Default::default()
    };
    assert_eq!(
        provision(&config, &cache).await.unwrap(),
        binary.canonicalize().unwrap()
    );
    assert!(!cache.exists());
    config.dolt_binary = None;
    config.cache_dir = Some(temporary.path().join("configured-cache"));
    let invalid = config
        .cache_dir
        .as_ref()
        .unwrap()
        .join(DOLT_VERSION)
        .join(BUNDLED_ASSET.target);
    private_directory(&invalid).unwrap();
    fs::write(invalid.join("sentinel"), b"existing data").unwrap();
    assert!(
        provision(&config, &cache)
            .await
            .unwrap_err()
            .to_string()
            .contains("cache is invalid")
    );
    assert_eq!(
        fs::read(invalid.join("sentinel")).unwrap(),
        b"existing data"
    );
    assert!(!cache.exists());
}

#[tokio::test]
async fn concurrent_cold_offline_extraction_activates_once_and_preserves_notices() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let fixture = &*VALID_FIXTURE;
    let config = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let (first, second) = tokio::join!(
        provision_managed(
            &config,
            &cache,
            fixture.spec(),
            Cow::Borrowed(&fixture.bytes)
        ),
        provision_managed(
            &config,
            &cache,
            fixture.spec(),
            Cow::Borrowed(&fixture.bytes)
        ),
    );
    let installed = first.unwrap();
    assert_eq!(installed, second.unwrap());
    let inode = fs::metadata(&installed).unwrap().ino();
    assert_eq!(
        provision_managed(
            &config,
            &cache,
            fixture.spec(),
            Cow::Borrowed(&fixture.bytes)
        )
        .await
        .unwrap(),
        installed
    );
    assert_eq!(fs::metadata(&installed).unwrap().ino(), inode);
    assert_eq!(fs::read(installed).unwrap(), SCRIPT);
    assert_eq!(
        fs::read(cache.join(DOLT_VERSION).join("fixture-target/LICENSES")).unwrap(),
        LICENSE
    );
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 1);
}

#[tokio::test]
async fn warm_verification_does_not_wait_for_the_installation_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let fixture = &*VALID_FIXTURE;
    let config = MemoryConfig::default();
    let installed = provision_managed(
        &config,
        &cache,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
    )
    .await
    .unwrap();
    let lock = cache_lock(&cache, Duration::from_secs(1)).await.unwrap();

    let warm = tokio::time::timeout(
        Duration::from_secs(2),
        provision_managed(
            &config,
            &cache,
            fixture.spec(),
            Cow::Borrowed(&fixture.bytes),
        ),
    )
    .await
    .expect("warm verification must not wait for installation authority")
    .unwrap();

    assert_eq!(warm, installed);
    drop(lock);
}

async fn observed_stages(progress: &mut crate::MemoryOpenProgress) -> Vec<MemoryOpenStage> {
    let mut stages = Vec::new();
    while let Some(stage) = progress.recv().await {
        stages.push(stage);
    }
    stages
}

#[tokio::test]
async fn observed_provision_reports_actual_cold_warm_and_failure_stages() {
    let root = tempfile::tempdir().unwrap();
    let fixture = &*VALID_FIXTURE;
    let config = MemoryConfig {
        offline: true,
        ..Default::default()
    };

    let cold_cache = root.path().join("cold");
    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    let cold = provision_managed_observed(
        &config,
        &cold_cache,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
        &mut reporter,
    )
    .await;
    drop(reporter);
    assert!(cold.is_ok());
    assert_eq!(
        observed_stages(&mut progress).await,
        [
            MemoryOpenStage::WaitingForRuntimeCache,
            MemoryOpenStage::ExtractingEmbeddedRuntime,
            MemoryOpenStage::CheckingRuntimeVersion,
        ]
    );

    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    let warm = provision_managed_observed(
        &config,
        &cold_cache,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
        &mut reporter,
    )
    .await;
    drop(reporter);
    assert!(warm.is_ok());
    assert_eq!(
        observed_stages(&mut progress).await,
        [
            MemoryOpenStage::VerifyingRuntimeCache,
            MemoryOpenStage::CheckingRuntimeVersion,
        ]
    );

    let corrupt_cached_entry = root.path().join("corrupt-cached-entry");
    let binary = provision_managed(
        &config,
        &corrupt_cached_entry,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
    )
    .await
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let mut corrupted = SCRIPT.to_vec();
    corrupted[0] = b'!';
    executable(&binary, &corrupted);
    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    let corrupt = provision_managed_observed(
        &config,
        &corrupt_cached_entry,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
        &mut reporter,
    )
    .await;
    drop(reporter);
    assert!(
        corrupt
            .unwrap_err()
            .to_string()
            .contains("cache is invalid")
    );
    assert_eq!(
        observed_stages(&mut progress).await,
        [MemoryOpenStage::VerifyingRuntimeCache]
    );

    let corrupt_cache = root.path().join("corrupt");
    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    let corrupt = provision_managed_observed(
        &config,
        &corrupt_cache,
        fixture.spec(),
        Cow::Owned(vec![b'!'; fixture.bytes.len()]),
        &mut reporter,
    )
    .await;
    drop(reporter);
    assert!(corrupt.is_err());
    assert_eq!(
        observed_stages(&mut progress).await,
        [
            MemoryOpenStage::WaitingForRuntimeCache,
            MemoryOpenStage::ExtractingEmbeddedRuntime,
        ]
    );

    let mismatch_cache = root.path().join("mismatch");
    static MISMATCH: std::sync::LazyLock<Fixture> = std::sync::LazyLock::new(|| {
        let version = b"#!/bin/sh\nprintf 'dolt version 2.3.2\\n'\n";
        let mut fixture = Fixture::new(|entries| entries[2].3 = version.to_vec());
        fixture.binary_digest = digest(version);
        fixture.binary_bytes = version.len() as u64;
        fixture
    });
    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    let mismatch = provision_managed_observed(
        &config,
        &mismatch_cache,
        MISMATCH.spec(),
        Cow::Borrowed(&MISMATCH.bytes),
        &mut reporter,
    )
    .await;
    drop(reporter);
    assert!(mismatch.is_err());
    assert_eq!(
        observed_stages(&mut progress).await,
        [
            MemoryOpenStage::WaitingForRuntimeCache,
            MemoryOpenStage::ExtractingEmbeddedRuntime,
            MemoryOpenStage::CheckingRuntimeVersion,
        ]
    );

    let explicit = root.path().join("explicit-dolt");
    executable(&explicit, SCRIPT);
    let explicit_config = MemoryConfig {
        dolt_binary: Some(explicit.clone()),
        ..config.clone()
    };
    let (mut progress, mut reporter) = crate::progress::ProgressReporter::observed();
    assert_eq!(
        provision_observed(&explicit_config, root.path(), &mut reporter)
            .await
            .expect("explicit binary should pass the version probe"),
        explicit
            .canonicalize()
            .expect("explicit binary should have a canonical path")
    );
    drop(reporter);
    assert_eq!(
        observed_stages(&mut progress).await,
        [MemoryOpenStage::CheckingRuntimeVersion]
    );
}

#[tokio::test]
async fn corrupt_embedded_bytes_never_activate_and_valid_retry_succeeds() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let fixture = &*VALID_FIXTURE;
    let config = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    for bytes in [
        vec![],
        vec![b'!'; fixture.bytes.len()],
        fixture.bytes[..fixture.bytes.len() - 1].to_vec(),
    ] {
        assert!(
            provision_managed(&config, &cache, fixture.spec(), Cow::Owned(bytes))
                .await
                .is_err()
        );
        assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
    }
    let binary = provision_managed(
        &config,
        &cache,
        fixture.spec(),
        Cow::Borrowed(&fixture.bytes),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(&binary).unwrap(), SCRIPT);
    // A corrupt bundle cannot trigger replacement or destructive repair of an
    // already verified immutable cache.
    assert_eq!(
        provision_managed(&config, &cache, fixture.spec(), Cow::Borrowed(b"bad"))
            .await
            .unwrap(),
        binary
    );
}

#[tokio::test]
async fn cancellation_retains_stage_and_lock_until_real_extraction_stops() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let directory = cache.clone();
    let (started, received) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        provision_with_extractor(
            &MemoryConfig {
                offline: true,
                ..Default::default()
            },
            &directory,
            VALID_FIXTURE.spec(),
            Cow::Borrowed(&VALID_FIXTURE.bytes),
            move |bytes, candidate, asset| {
                private_directory(candidate)?;
                started.send(candidate.to_path_buf()).unwrap();
                wait.recv_timeout(Duration::from_secs(10))
                    .context("release extraction fixture")?;
                extract(bytes, candidate, asset)
            },
        )
        .await
    });
    let stage = tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        stage.exists(),
        "cancelled caller cannot delete the worker's stage"
    );
    assert!(
        cache_lock(&cache, Duration::from_millis(30)).await.is_err(),
        "worker retains installation lock until its filesystem writes stop"
    );
    release.send(()).unwrap();
    let _lock = cache_lock(&cache, Duration::from_secs(10)).await.unwrap();
    assert!(!stage.exists());
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
    drop(_lock);
    let installed = provision_managed(
        &MemoryConfig::default(),
        &cache,
        VALID_FIXTURE.spec(),
        Cow::Borrowed(&VALID_FIXTURE.bytes),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(installed).unwrap(), SCRIPT);
}

async fn paused_probe(
    cache: PathBuf,
) -> (tokio::task::JoinHandle<Result<PathBuf>>, PathBuf, PathBuf) {
    const CONTROLLED: &[u8] = b"#!/bin/sh\nprintf started > \"$TMPDIR/started\"\nIFS= read -r token < \"$TMPDIR/release\" || exit 17\nprintf 'dolt version 2.3.3\\n'\n";
    static FIXTURE: std::sync::LazyLock<Fixture> = std::sync::LazyLock::new(|| {
        let mut fixture = Fixture::new(|entries| entries[2].3 = CONTROLLED.to_vec());
        fixture.binary_digest = digest(CONTROLLED);
        fixture.binary_bytes = CONTROLLED.len() as u64;
        fixture
    });
    let directory = cache;
    let (staged, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        provision_with_extractor(
            &MemoryConfig::default(),
            &directory,
            FIXTURE.spec(),
            Cow::Borrowed(&FIXTURE.bytes),
            move |bytes, candidate, asset| {
                extract(bytes, candidate, asset)?;
                let home = candidate.parent().unwrap().join("probe");
                prepare_private_home(&home)?;
                nix::unistd::mkfifo(
                    &home.join("tmp/release"),
                    nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
                )?;
                staged.send((candidate.to_owned(), home)).unwrap();
                Ok(())
            },
        )
        .await
    });
    let (candidate, home) = tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !home.join("tmp/started").try_exists().unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "real version process never reached its FIFO barrier"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    (task, candidate, home)
}

#[tokio::test]
async fn cancellation_during_actual_probe_retains_stage_and_lock_but_never_activates() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    let (task, candidate, home) = paused_probe(cache.clone()).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        candidate.exists(),
        "live version process must retain its candidate"
    );
    assert!(
        home.exists(),
        "live version process must retain its private home"
    );
    assert!(
        cache_lock(&cache, Duration::from_millis(30)).await.is_err(),
        "live version process must retain installation authority"
    );
    let release = nix::fcntl::open(
        &home.join("tmp/release"),
        nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
        nix::sys::stat::Mode::empty(),
    )
    .unwrap();
    nix::unistd::write(&release, b"continue\n").unwrap();
    let lock = cache_lock(&cache, Duration::from_secs(10)).await.unwrap();
    drop(release);
    assert!(!candidate.exists());
    assert!(!home.exists());
    assert_eq!(
        fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(),
        0,
        "canceled caller cannot publish after a successful probe"
    );
    drop(lock);
    let installed = provision_managed(
        &MemoryConfig::default(),
        &cache,
        VALID_FIXTURE.spec(),
        Cow::Borrowed(&VALID_FIXTURE.bytes),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(installed).unwrap(), SCRIPT);
}

#[test]
fn runtime_destruction_during_probe_retains_process_resources_until_exit_without_activation() {
    let root = crate::test_support::tempdir().unwrap();
    let cache = root.path().join("cache");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_task, candidate, home) = runtime.block_on(paused_probe(cache.clone()));
    drop(runtime);
    assert!(
        candidate.exists(),
        "destroying the caller executor removed the running probe's candidate"
    );
    assert!(
        home.exists(),
        "destroying the caller executor removed the running probe's home"
    );
    let observer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(
        observer
            .block_on(cache_lock(&cache, Duration::from_millis(30)))
            .is_err(),
        "destroying the caller executor released a live probe's cache lease"
    );
    let release = nix::fcntl::open(
        &home.join("tmp/release"),
        nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
        nix::sys::stat::Mode::empty(),
    )
    .unwrap();
    nix::unistd::write(&release, b"continue\n").unwrap();
    let _lock = observer
        .block_on(cache_lock(&cache, Duration::from_secs(10)))
        .unwrap();
    drop(release);
    assert!(!candidate.exists());
    assert!(!home.exists());
    assert_eq!(
        fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(),
        0,
        "a destroyed caller cannot activate a successfully completed probe"
    );
}

#[tokio::test]
async fn actual_embedded_engine_starts_offline_from_empty_cache_and_reuses_verified_payloads() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("empty offline café cache");
    let config = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    assert!(!cache.exists());
    let binary = provision(&config, &cache)
        .await
        .expect("embedded Dolt must support offline first use");
    let asset = BUNDLED_ASSET;
    verify_payload(
        &binary,
        asset.executable_bytes,
        asset.executable_sha256,
        true,
    )
    .unwrap();
    verify_payload(
        &binary.parent().unwrap().join("LICENSES"),
        asset.license_bytes,
        asset.license_sha256,
        false,
    )
    .unwrap();
    let inode = fs::metadata(&binary).unwrap().ino();
    assert_eq!(provision(&config, &cache).await.unwrap(), binary);
    assert_eq!(fs::metadata(&binary).unwrap().ino(), inode);
    verify_version(&binary, &temporary.path().join("actual engine café"))
        .await
        .unwrap();
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 1);
}

#[tokio::test]
async fn corrupt_actual_embedded_archive_is_rejected_without_executing_or_activating() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let mut bytes = EMBEDDED_ARCHIVE.to_vec();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x80;
    let failure = provision_managed(
        &MemoryConfig::default(),
        &cache,
        BUNDLED_ASSET,
        Cow::Owned(bytes),
    )
    .await
    .unwrap_err();
    assert!(failure.to_string().contains("checksum mismatch"));
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
}
