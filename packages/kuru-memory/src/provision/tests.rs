use super::*;
use flate2::{Compression, write::GzEncoder};
use std::os::unix::fs::symlink;

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
            license_digest: digest(LICENSE),
            bytes,
        }
    }

    fn spec(&self) -> Asset<'_> {
        Asset {
            target: "fixture-target",
            stem: "fixture",
            url: "https://example.invalid/fixture.tar.gz",
            compressed_bytes: self.bytes.len() as u64,
            archive_sha256: &self.archive_digest,
            expanded_bytes: self.expanded,
            executable_bytes: SCRIPT.len() as u64,
            executable_sha256: &self.binary_digest,
            license_bytes: LICENSE.len() as u64,
            license_sha256: &self.license_digest,
        }
    }

    fn extract(&self, directory: &Path) -> Result<PathBuf> {
        let archive = directory.join("asset.tar.gz");
        fs::write(&archive, &self.bytes)?;
        let candidate = directory.join("candidate");
        extract(&archive, &candidate, self.spec())?;
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
    let temporary = tempfile::tempdir().unwrap();
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
    let archive = temporary.path().join("archive");
    let destination = temporary.path().join("candidate");
    fs::write(&archive, &original.bytes).unwrap();
    let mut spec = original.spec();
    spec.archive_sha256 = "incorrect";
    assert!(extract(&archive, &destination, spec).is_err());
    spec = original.spec();
    spec.expanded_bytes -= 1;
    assert!(extract(&archive, &destination, spec).is_err());
    spec.expanded_bytes = MAX_EXPANDED + 1;
    assert!(extract(&archive, &destination, spec).is_err());
    spec = original.spec();
    spec.compressed_bytes += 1;
    assert!(extract(&archive, &destination, spec).is_err());
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

#[tokio::test]
async fn stable_lock_waits_times_out_and_does_not_delete_a_held_inode() {
    let temporary = tempfile::tempdir().unwrap();
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
    fs::remove_dir(home.join("root/.dolt")).unwrap();
    symlink(temporary.path(), home.join("root/.dolt")).unwrap();
    assert!(prepare_private_home(&home).is_err());
}

#[tokio::test]
async fn explicit_binary_wins_and_missing_offline_cache_is_actionable() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let mut config = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let failure = provision(&config, &cache).await.unwrap_err().to_string();
    assert!(failure.contains("run once online"));
    let binary = temporary.path().join("explicit-dolt");
    executable(&binary, SCRIPT);
    config.dolt_binary = Some(binary.clone());
    assert_eq!(
        provision(&config, &cache).await.unwrap(),
        binary.canonicalize().unwrap()
    );
    config.dolt_binary = None;
    config.cache_dir = Some(temporary.path().join("configured-cache"));
    assert!(
        provision(&config, &cache)
            .await
            .unwrap_err()
            .to_string()
            .contains("configured-cache")
    );
    let asset = host_asset().unwrap();
    let invalid = config
        .cache_dir
        .as_ref()
        .unwrap()
        .join(DOLT_VERSION)
        .join(asset.target);
    private_directory(&invalid).unwrap();
    assert!(
        provision(&config, &cache)
            .await
            .unwrap_err()
            .to_string()
            .contains("cache is invalid")
    );
}

async fn http_response(
    body: Vec<u8>,
    content_length: Option<usize>,
    status: &str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let task = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut buffer = [0_u8; 512];
            let size = socket.read(&mut buffer).await.unwrap();
            assert!(size > 0, "fixture client closed before its HTTP headers");
            request.extend_from_slice(&buffer[..size]);
            assert!(request.len() <= 4096);
        }
        let header = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\n{}\r\n",
            content_length
                .map(|size| format!("Content-Length: {size}\r\n"))
                .unwrap_or_default()
        );
        let _ = socket.write_all(header.as_bytes()).await;
        let _ = socket.write_all(&body).await;
        let _ = socket.shutdown().await;
    });
    (format!("http://{address}/dolt.tar.gz"), task)
}

#[tokio::test]
async fn download_checks_status_declared_streamed_sizes_and_checksum_before_installation() {
    let fixture = Fixture::new(|_| {});
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    for (body, length, status, accepted) in [
        (
            fixture.bytes.clone(),
            Some(fixture.bytes.len()),
            "200 OK",
            true,
        ),
        (fixture.bytes.clone(), None, "200 OK", true),
        (
            fixture.bytes.clone(),
            Some(fixture.bytes.len() + 1),
            "200 OK",
            false,
        ),
        (vec![], None, "200 OK", false),
        (vec![b'x'; fixture.bytes.len() + 1], None, "200 OK", false),
        (vec![b'x'; fixture.bytes.len()], None, "200 OK", false),
        (vec![], Some(0), "404 Not Found", false),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let (url, server) = http_response(body, length, status).await;
        let result = download(
            &client,
            &url,
            &temporary.path().join("archive"),
            fixture.spec(),
        )
        .await;
        assert_eq!(result.is_ok(), accepted, "{result:?}");
        server.await.unwrap();
    }
    let temporary = tempfile::tempdir().unwrap();
    let mut oversized = fixture.spec();
    oversized.compressed_bytes = MAX_COMPRESSED + 1;
    assert!(
        download(
            &client,
            fixture.spec().url,
            &temporary.path().join("oversized"),
            oversized
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn first_use_downloads_once_then_concurrent_and_offline_open_reuse_the_atomic_cache() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let fixture = &*VALID_FIXTURE;
    let (url, server) =
        http_response(fixture.bytes.clone(), Some(fixture.bytes.len()), "200 OK").await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let config = MemoryConfig::default();
    let (first, second) = tokio::join!(
        provision_managed(&config, &cache, fixture.spec(), &client, &url),
        provision_managed(&config, &cache, fixture.spec(), &client, &url),
    );
    let installed = first.unwrap();
    assert_eq!(installed, second.unwrap());
    server.await.unwrap();
    let offline = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    assert_eq!(
        provision_managed(&offline, &cache, fixture.spec(), &client, &url)
            .await
            .unwrap(),
        installed
    );
    assert_eq!(fs::read(installed).unwrap(), SCRIPT);
    assert_eq!(
        fs::read(cache.join(DOLT_VERSION).join("fixture-target/LICENSES")).unwrap(),
        LICENSE
    );
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 1);
}

#[tokio::test]
async fn failed_download_leaves_no_active_or_partial_install_and_can_be_retried() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let fixture = &*VALID_FIXTURE;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let (url, server) = http_response(vec![b'!'; fixture.bytes.len()], None, "200 OK").await;
    assert!(
        provision_managed(
            &MemoryConfig::default(),
            &cache,
            fixture.spec(),
            &client,
            &url
        )
        .await
        .is_err()
    );
    server.await.unwrap();
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
    let (url, server) = http_response(fixture.bytes.clone(), None, "200 OK").await;
    provision_managed(
        &MemoryConfig::default(),
        &cache,
        fixture.spec(),
        &client,
        &url,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert!(
        cache
            .join(DOLT_VERSION)
            .join("fixture-target/dolt")
            .exists()
    );
}

#[tokio::test]
async fn cancellation_during_download_releases_lock_and_removes_unactivated_stage() {
    let temporary = tempfile::tempdir().unwrap();
    let cache = temporary.path().join("cache");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/stall", listener.local_addr().unwrap());
    let (started, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut byte = [0_u8; 1];
        socket.read_exact(&mut byte).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let directory = cache.clone();
    let task = tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        provision_managed(
            &MemoryConfig::default(),
            &directory,
            VALID_FIXTURE.spec(),
            &client,
            &url,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    server.abort();
    let _ = server.await;
    assert_eq!(fs::read_dir(cache.join(DOLT_VERSION)).unwrap().count(), 0);
    cache_lock(&cache, Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn real_prefetched_engine_runs_from_a_verified_cache_with_offline_policy() {
    // The package's mise pretest task provisions this cache. A missing runtime is
    // a required integration failure, never a reason to skip or substitute SQLite.
    let cache = std::env::var_os("KURU_DOLT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("kuru-dolt-test-cache"));
    let config = MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let binary = provision(&config, &cache)
        .await
        .expect("mise memory pretest must provision the pinned real Dolt engine");
    let temporary = tempfile::tempdir().unwrap();
    verify_version(&binary, &temporary.path().join("actual engine café"))
        .await
        .unwrap();
    let asset = host_asset().unwrap();
    assert_eq!(fs::metadata(&binary).unwrap().len(), asset.executable_bytes);
    assert_eq!(
        fs::metadata(binary.parent().unwrap().join("LICENSES"))
            .unwrap()
            .len(),
        asset.license_bytes
    );
}
