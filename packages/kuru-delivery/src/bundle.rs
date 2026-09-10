//! Verified, hash-addressed build inputs. This helper never executes an archive
//! or depends on the runtime that will eventually consume its bytes.

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{
    Directory as CheckedDirectory, NameRetention, Privacy, Publication, require_private,
    seal_private,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{File, TryLockError},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const LOCK_NAME: &str = ".prepare.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug)]
pub struct PrepareOptions {
    pub manifest: PathBuf,
    pub target: String,
    pub bundle_dir: PathBuf,
    pub archive: Option<PathBuf>,
    pub offline: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    version: String,
    upstream_commit: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    target: String,
    stem: String,
    format: String,
    executable_name: String,
    url: String,
    compressed_bytes: u64,
    archive_sha256: String,
    expanded_bytes: u64,
    executable_bytes: u64,
    executable_sha256: String,
    license_bytes: u64,
    license_sha256: String,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn manifest(path: &Path, target: &str) -> Result<Asset> {
    let (directory, name) = input_parent(path)?;
    let mut file = directory.read(&name, false)?;
    ensure!(
        file.metadata()?.len() <= MAX_MANIFEST,
        "bundle manifest exceeds 64 KiB"
    );
    let mut bytes = Vec::new();
    (&mut file).take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "bundle manifest exceeds 64 KiB"
    );
    directory.verify(&name, &file, false)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).context("parse Dolt asset manifest")?;
    ensure!(
        manifest.schema_version == 1,
        "unsupported Dolt asset manifest schema"
    );
    crate::archive::checked_version(&manifest.version)?;
    ensure!(
        hex(&manifest.upstream_commit, 40),
        "invalid upstream commit in bundle manifest"
    );
    ensure!(
        !manifest.assets.is_empty() && manifest.assets.len() <= 32,
        "bundle manifest must contain 1–32 targets"
    );
    let mut targets = HashSet::new();
    for asset in &manifest.assets {
        ensure!(
            identifier(&asset.target) && identifier(&asset.stem),
            "invalid target or stem in bundle manifest"
        );
        ensure!(
            matches!(
                (asset.format.as_str(), asset.executable_name.as_str()),
                ("tar.gz", "dolt") | ("zip", "dolt.exe")
            ),
            "unsupported bundle format or executable name"
        );
        ensure!(
            targets.insert(&asset.target),
            "duplicate bundle target in manifest"
        );
        ensure!(
            asset.compressed_bytes > 0 && asset.compressed_bytes <= MAX_ARCHIVE,
            "bundle archive size must be within 64 MiB"
        );
        ensure!(
            asset.expanded_bytes > 0
                && asset.expanded_bytes <= MAX_EXPANDED
                && asset.executable_bytes > 0
                && asset.license_bytes > 0
                && asset
                    .executable_bytes
                    .checked_add(asset.license_bytes)
                    .is_some_and(|size| size <= asset.expanded_bytes),
            "invalid bounded payload sizes in bundle manifest"
        );
        ensure!(
            [
                &asset.archive_sha256,
                &asset.executable_sha256,
                &asset.license_sha256
            ]
            .into_iter()
            .all(|value| hex(value, 64)),
            "invalid SHA-256 in bundle manifest"
        );
        let url = url::Url::parse(&asset.url).context("invalid bundle archive URL")?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "bundle archive URL must be HTTPS without credentials, query or fragment"
        );
    }
    manifest
        .assets
        .into_iter()
        .find(|asset| asset.target == target)
        .with_context(|| {
            format!("bundle manifest has no archive for target {target}; host fallback is disabled")
        })
}

/// Resolve the conventional default from the manifest's owning workspace. An
/// override is explicit and must be absolute; neither path depends on Cargo's
/// host compilation target.
pub fn bundle_directory(manifest: &Path, override_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_dir {
        ensure!(
            path.is_absolute(),
            "bundle directory override must be absolute"
        );
        return Ok(path);
    }
    let manifest = absolute(manifest)?;
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("manifest must live under packages/kuru-memory/support")?;
    ensure!(
        manifest.ends_with("packages/kuru-memory/support/dolt-assets.json"),
        "provide --bundle-dir for a manifest outside packages/kuru-memory/support"
    );
    Ok(workspace.join("target/kuru-bundles"))
}

pub async fn prepare(options: &PrepareOptions) -> Result<PathBuf> {
    ensure!(
        options.bundle_dir.is_absolute(),
        "bundle directory must be absolute"
    );
    let target = if options.target == "host" {
        crate::archive::host_target()?
    } else {
        &options.target
    };
    let asset = manifest(&options.manifest, target)?;
    prepare_asset(options, &asset, None).await
}

async fn prepare_asset(
    options: &PrepareOptions,
    asset: &Asset,
    client: Option<&reqwest::Client>,
) -> Result<PathBuf> {
    let directory =
        Directory::open(&options.bundle_dir, true, true).context("open bundle cache directory")?;
    let lock = directory
        .lock()
        .await
        .context("acquire stable bundle cache lock")?;
    let name = format!("{}.archive", asset.archive_sha256);
    let destination = directory.path().join(&name);
    match directory.read(OsStr::new(&name), true) {
        Ok(mut file) => {
            validate(&mut file, asset)?;
            directory.verify(OsStr::new(&name), &file, true)?;
            return Ok(destination);
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => {
            return Err(error).context("existing prepared bundle is unsafe; it was preserved");
        }
    }
    ensure!(
        options.archive.is_some() || !options.offline,
        "prepared bundle is missing in offline mode; import the pinned archive with --archive"
    );
    let mut staging = tempfile::Builder::new()
        .prefix(".bundle-")
        .tempfile_in(directory.path())?;
    if let Some(path) = &options.archive {
        let (source, name) = input_parent(path).context("open local archive directory")?;
        let mut file = source.read(&name, false).context("open local archive")?;
        ensure!(
            file.metadata()?.len() == asset.compressed_bytes,
            "bundle archive size mismatch"
        );
        let mut digest = Sha256::new();
        let mut size = 0;
        let mut buffer = [0_u8; 65536];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            size += read as u64;
            ensure!(
                size <= asset.compressed_bytes,
                "bundle archive exceeds pinned size"
            );
            digest.update(&buffer[..read]);
            staging.write_all(&buffer[..read])?;
        }
        verify_digest(size, &digest.finalize(), asset)?;
        source
            .verify(&name, &file, false)
            .context("verify local archive identity")?;
    } else {
        let default_client = reqwest::Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(120))
            .build()?;
        download(
            client.unwrap_or(&default_client),
            asset,
            staging.as_file_mut(),
        )
        .await?;
    }
    seal_private(staging.as_file(), false)?;
    staging.as_file().sync_all()?;
    directory
        .revalidate()
        .context("verify cache directory before publication")?;
    directory.verify(OsStr::new(LOCK_NAME), &lock, true)?;
    let staged_name = staging
        .path()
        .file_name()
        .context("missing staged bundle filename")?;
    directory
        .verify(staged_name, staging.as_file(), true)
        .context("verify staged archive before publication")?;
    directory
        .native
        .publish_file(
            &directory.native,
            staged_name,
            staging.as_file(),
            OsStr::new(&name),
            Publication::New,
        )
        .context("publish prepared bundle without replacement")?;
    directory.verify(OsStr::new(&name), staging.as_file(), true)?;
    Ok(destination)
}

async fn download(client: &reqwest::Client, asset: &Asset, output: &mut File) -> Result<()> {
    let mut response = client.get(&asset.url).send().await?.error_for_status()?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == asset.compressed_bytes),
        "bundle archive size mismatch"
    );
    let mut size = 0_u64;
    let mut digest = Sha256::new();
    while let Some(chunk) = response.chunk().await? {
        size += chunk.len() as u64;
        ensure!(
            size <= asset.compressed_bytes,
            "bundle archive exceeds pinned size"
        );
        digest.update(&chunk);
        // Synchronous bounded chunk writes have no worker that can outlive the
        // staging file or lock when the surrounding download future is cancelled.
        output.write_all(&chunk)?;
    }
    verify_digest(size, &digest.finalize(), asset)
}

fn verify_digest(size: u64, digest: &[u8], asset: &Asset) -> Result<()> {
    ensure!(
        size == asset.compressed_bytes,
        "bundle archive size mismatch"
    );
    let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    ensure!(
        digest == asset.archive_sha256,
        "bundle archive checksum mismatch; existing inputs were preserved"
    );
    Ok(())
}

fn validate(file: &mut File, asset: &Asset) -> Result<()> {
    ensure!(
        file.metadata()?.len() == asset.compressed_bytes,
        "prepared bundle size mismatch; corrupt cache was preserved"
    );
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        ensure!(
            size <= asset.compressed_bytes,
            "prepared bundle exceeds pinned size"
        );
        digest.update(&buffer[..read]);
    }
    verify_digest(size, &digest.finalize(), asset)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::ParentDir)),
        "bundle paths must not contain parent traversal"
    );
    Ok(path)
}

fn input_parent(path: &Path) -> Result<(Directory, std::ffi::OsString)> {
    let path = absolute(path)?;
    let name = path
        .file_name()
        .context("bundle input needs a filename")?
        .to_owned();
    let directory = Directory::open(
        path.parent().context("bundle input needs a parent")?,
        false,
        false,
    )?;
    Ok((directory, name))
}

struct Directory {
    native: CheckedDirectory,
    private: bool,
}
impl Directory {
    fn open(path: &Path, create: bool, owner_only: bool) -> Result<Self> {
        let path = absolute(path)?;
        let native = if create {
            ensure!(owner_only, "new bundle directories must be private");
            CheckedDirectory::ensure_private(&path)?
        } else {
            CheckedDirectory::open(
                &path,
                if owner_only {
                    Privacy::OwnerOnly
                } else {
                    Privacy::Inherited
                },
                NameRetention::Movable,
            )?
        };
        Ok(Self {
            native,
            private: owner_only,
        })
    }
    fn path(&self) -> &Path {
        self.native.path()
    }
    fn revalidate(&self) -> Result<()> {
        let current = Self::open(self.path(), false, self.private)?;
        ensure!(
            current.native.identity() == self.native.identity(),
            "bundle directory or ancestor was replaced"
        );
        Ok(())
    }
    fn read(&self, name: &OsStr, owner_only: bool) -> Result<File> {
        let file = self.native.read(name)?;
        if owner_only {
            require_private(&file)?;
        }
        Ok(file)
    }
    fn verify(&self, name: &OsStr, held: &File, owner_only: bool) -> Result<()> {
        if owner_only {
            require_private(held)?;
        }
        self.native.verify(name, held)?;
        Ok(())
    }
    async fn lock(&self) -> Result<File> {
        let pinned =
            CheckedDirectory::open(self.path(), Privacy::OwnerOnly, NameRetention::Pinned)?;
        let lock = pinned
            .lock_file(OsStr::new(LOCK_NAME))
            .context("create or open stable lock file")?;
        let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "timed out waiting for bundle preparation; held lock was preserved"
                    );
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => bail!("lock bundle preparation: {error}"),
            }
        }
        self.verify(OsStr::new(LOCK_NAME), &lock, true)
            .context("verify acquired stable lock identity")?;
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_platform::fs::regular_file_info;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn asset(bytes: &[u8]) -> Asset {
        Asset {
            target: "test-target".into(),
            stem: "dolt-fixture".into(),
            format: "tar.gz".into(),
            executable_name: "dolt".into(),
            url: String::new(),
            compressed_bytes: bytes.len() as u64,
            archive_sha256: crate::archive::digest(bytes),
            expanded_bytes: 100,
            executable_bytes: 60,
            executable_sha256: "a".repeat(64),
            license_bytes: 20,
            license_sha256: "b".repeat(64),
        }
    }
    fn options(path: &Path) -> PrepareOptions {
        PrepareOptions {
            manifest: path.join("unused-manifest"),
            target: "test-target".into(),
            bundle_dir: path.join("cache"),
            archive: None,
            offline: false,
        }
    }
    fn assert_clean(path: &Path) {
        let names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsStr::new(LOCK_NAME)]);
        let lock = File::options()
            .read(true)
            .write(true)
            .open(path.join(LOCK_NAME))
            .unwrap();
        lock.try_lock().unwrap();
    }

    #[tokio::test]
    async fn streamed_download_validates_size_hash_and_status_before_publication() {
        let expected = b"verified network archive";
        for (body, status, succeeds) in [
            (expected.as_slice(), 200, true),
            (b"wrongxxx network archive".as_slice(), 200, false),
            (b"short".as_slice(), 200, false),
            (
                b"verified network archive with extra bytes".as_slice(),
                200,
                false,
            ),
            (expected.as_slice(), 503, false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                let mut response = format!("HTTP/1.1 {status} Fixture\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", body.len()).into_bytes();
                response.extend_from_slice(body);
                response.extend_from_slice(b"\r\n0\r\n\r\n");
                let _ = socket.write_all(&response).await;
            });
            let mut asset = asset(expected);
            asset.url = format!("http://{address}/archive");
            let options = options(root.path());
            // Only this private transport fixture supplies an HTTP client;
            // public prepare always validates HTTPS and creates https_only.
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let result = prepare_asset(&options, &asset, Some(&client)).await;
            server.abort();
            let _ = server.await;
            if succeeds {
                assert_eq!(fs::read(result.unwrap()).unwrap(), expected);
            } else {
                assert!(result.is_err());
                assert_clean(&options.bundle_dir);
            }
        }
    }

    #[tokio::test]
    async fn cancelling_a_stalled_download_removes_stage_and_releases_stable_lock() {
        let root = tempfile::tempdir().unwrap();
        let options = options(root.path());
        let cache = options.bundle_dir.clone();
        let expected = b"verified network archive";
        let mut asset = asset(expected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        asset.url = format!("http://{}/archive", listener.local_addr().unwrap());
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() < 8192);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nveri\r\n").await.unwrap();
            tokio::time::timeout(Duration::from_secs(10), released)
                .await
                .unwrap()
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let download =
            tokio::spawn(async move { prepare_asset(&options, &asset, Some(&client)).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let staged = fs::read_dir(&cache)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .any(|entry| {
                        entry.file_name().to_string_lossy().starts_with(".bundle-")
                            && entry.metadata().unwrap().len() == 4
                    });
                if staged {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fixture never observed the written partial download");
        let before = regular_file_info(&File::open(cache.join(LOCK_NAME)).unwrap())
            .unwrap()
            .identity;
        download.abort();
        assert!(download.await.unwrap_err().is_cancelled());
        assert_clean(&cache);
        let after = regular_file_info(&File::open(cache.join(LOCK_NAME)).unwrap())
            .unwrap()
            .identity;
        assert_eq!(before, after);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn replaced_lock_and_directory_are_rejected_without_adopting_new_objects() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cache");
        let directory = Directory::open(&path, true, true).unwrap();
        let held = directory.lock().await.unwrap();
        let old_lock = path.join("old-lock");
        #[cfg(windows)]
        {
            // The native pinned handle denies substitution instead of detecting
            // it afterward. Both preserve one actual lock and the original bytes.
            assert!(fs::rename(path.join(LOCK_NAME), &old_lock).is_err());
            assert!(fs::rename(&path, root.path().join("retired")).is_err());
            directory
                .verify(OsStr::new(LOCK_NAME), &held, true)
                .unwrap();
            assert!(
                File::options()
                    .read(true)
                    .write(true)
                    .open(path.join(LOCK_NAME))
                    .unwrap()
                    .try_lock()
                    .is_err()
            );
            drop(held);
            let replacement = directory.lock().await.unwrap();
            directory
                .verify(OsStr::new(LOCK_NAME), &replacement, true)
                .unwrap();
        }
        #[cfg(unix)]
        {
            fs::rename(path.join(LOCK_NAME), &old_lock).unwrap();
            let replacement = File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.join(LOCK_NAME))
                .unwrap();
            replacement
                .set_permissions(fs::Permissions::from_mode(0o600))
                .unwrap();
            replacement.try_lock().unwrap();
            assert!(
                directory
                    .verify(OsStr::new(LOCK_NAME), &held, true)
                    .is_err()
            );
            assert!(
                File::options()
                    .read(true)
                    .write(true)
                    .open(&old_lock)
                    .unwrap()
                    .try_lock()
                    .is_err()
            );
            let old_directory = root.path().join("retired");
            fs::rename(&path, &old_directory).unwrap();
            Directory::open(&path, true, true).unwrap();
            assert!(directory.revalidate().is_err());
            assert!(old_directory.join("old-lock").is_file());
        }
    }
}
