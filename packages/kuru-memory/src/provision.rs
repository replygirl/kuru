//! Bounded, private installation of the exact supported full-Dolt engine.

pub use crate::catalog::DOLT_VERSION;
use crate::catalog::{Asset, BUNDLED_ASSET, EMBEDDED_ARCHIVE, MAX_COMPRESSED, MAX_EXPANDED};
use anyhow::{Context, Result, bail, ensure};
use flate2::bufread::GzDecoder;
use kuru_core::MemoryConfig;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::HashSet,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Cursor, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const LOCK_TIMEOUT: Duration = Duration::from_secs(180);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: u64 = 4096;

/// Extract the bundled engine or reuse its verified cache, including offline
/// first use. An explicit development binary still passes the exact version
/// guard; managed entries also pass their immutable payload checksums.
pub async fn provision(config: &MemoryConfig, default_cache: &Path) -> Result<PathBuf> {
    if let Some(binary) = &config.dolt_binary {
        checked_regular(binary, true)?;
        let binary = binary
            .canonicalize()
            .context("resolve configured Dolt executable")?;
        let home = tempfile::Builder::new()
            .prefix("kuru-dolt-version-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()?;
        verify_version(&binary, home.path()).await?;
        return Ok(binary);
    }
    provision_managed(
        config,
        default_cache,
        BUNDLED_ASSET,
        Cow::Borrowed(EMBEDDED_ARCHIVE),
    )
    .await
}

async fn provision_managed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
) -> Result<PathBuf> {
    provision_with_extractor(config, default_cache, asset, archive, extract).await
}

async fn provision_with_extractor(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    extractor: impl FnOnce(&[u8], &Path, Asset<'static>) -> Result<()> + Send + 'static,
) -> Result<PathBuf> {
    let cache = config.cache_dir.as_deref().unwrap_or(default_cache);
    private_directory(cache)?;
    let cache = cache.canonicalize()?;
    let _lock = cache_lock(&cache, LOCK_TIMEOUT).await?;
    let versions = cache.join(DOLT_VERSION);
    private_directory(&versions)?;
    let destination = versions.join(asset.target);
    if destination.try_exists()? {
        return verified_cache(&destination, asset).await.with_context(|| {
            format!(
                "Dolt cache is invalid at {}; preserve or remove that version directory and retry",
                destination.display()
            )
        });
    }
    // A dangling link also represents an existing unsafe destination.
    ensure!(
        fs::symlink_metadata(&destination).is_err(),
        "Dolt cache destination is not a new directory"
    );
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&versions)?;
    let candidate = staging.path().join("runtime");
    let candidate_path = candidate.clone();
    let (staging, _lock, extraction) = tokio::task::spawn_blocking(move || {
        let result = extractor(&archive, &candidate_path, asset);
        // Keep the stage and stable lock until the worker exits, even if its
        // caller is cancelled. Drop the stage before releasing the lock.
        (staging, _lock, result)
    })
    .await?;
    extraction?;
    let probe_home = staging.path().join("probe");
    verify_version(&candidate.join("dolt"), &probe_home).await?;
    activate(&candidate, &destination)?;
    Ok(destination.join("dolt"))
}

async fn verified_cache(directory: &Path, asset: Asset<'_>) -> Result<PathBuf> {
    check_directory(directory)?;
    let binary = directory.join("dolt");
    let executable = binary.clone();
    let executable_bytes = asset.executable_bytes;
    let executable_sha256 = asset.executable_sha256.to_owned();
    let licenses = directory.join("LICENSES");
    let license_bytes = asset.license_bytes;
    let license_sha256 = asset.license_sha256.to_owned();
    tokio::task::spawn_blocking(move || {
        verify_payload(&executable, executable_bytes, &executable_sha256, true)?;
        verify_payload(&licenses, license_bytes, &license_sha256, false)
    })
    .await??;
    let home = tempfile::Builder::new()
        .prefix("kuru-dolt-version-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?;
    verify_version(&binary, home.path()).await?;
    Ok(binary)
}

fn extract(archive: &[u8], destination: &Path, asset: Asset<'_>) -> Result<()> {
    ensure!(
        asset.compressed_bytes > 0 && asset.compressed_bytes <= MAX_COMPRESSED,
        "pinned Dolt archive exceeds compressed byte budget"
    );
    ensure!(
        archive.len() as u64 == asset.compressed_bytes,
        "bundled Dolt archive size does not match its pin"
    );
    ensure!(
        hex_digest(&Sha256::digest(archive)) == asset.archive_sha256,
        "bundled Dolt archive checksum mismatch; no payload was executed"
    );
    ensure!(
        asset.expanded_bytes <= MAX_EXPANDED,
        "pinned Dolt archive exceeds expanded byte budget"
    );
    // Bound decompression before tar sees extension headers or payload lengths.
    let mut expanded = Vec::new();
    let mut decoder = GzDecoder::new(Cursor::new(archive));
    (&mut decoder)
        .take(asset.expanded_bytes + 1)
        .read_to_end(&mut expanded)?;
    ensure!(
        expanded.len() as u64 == asset.expanded_bytes,
        "Dolt expanded archive size does not match its pin"
    );
    ensure!(
        Read::read(&mut decoder.into_inner(), &mut [0_u8; 1])? == 0,
        "Dolt archive contains trailing compressed data"
    );
    private_directory(destination)?;
    let mut archive = tar::Archive::new(Cursor::new(expanded.as_slice()));
    let directory = format!("{}/", asset.stem);
    let bin_directory = format!("{}/bin/", asset.stem);
    let executable = format!("{}/bin/dolt", asset.stem);
    let licenses = format!("{}/LICENSES", asset.stem);
    let mut seen = HashSet::new();
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        let path = entry.path_bytes().to_vec();
        ensure!(
            seen.insert(path.clone()),
            "Dolt archive contains duplicate entries"
        );
        ensure!(
            entry.header().as_gnu().is_some() && entry.header().link_name_bytes().is_none(),
            "Dolt archive contains unexpected metadata or links"
        );
        let (output, length, checksum, mode) =
            if path == directory.as_bytes() || path == bin_directory.as_bytes() {
                ensure!(
                    entry.header().entry_type().is_dir()
                        && entry.size() == 0
                        && entry.header().mode()? == 0o755,
                    "Dolt archive directory has an unexpected type, size or mode"
                );
                continue;
            } else if path == executable.as_bytes() {
                (
                    "dolt",
                    asset.executable_bytes,
                    asset.executable_sha256,
                    0o755,
                )
            } else if path == licenses.as_bytes() {
                ("LICENSES", asset.license_bytes, asset.license_sha256, 0o644)
            } else {
                bail!("Dolt archive contains an unexpected entry");
            };
        ensure!(
            entry.header().entry_type().is_file()
                && entry.size() == length
                && entry.header().mode()? == mode,
            "Dolt archive payload has an unexpected type, size or mode"
        );
        let path = destination.join(output);
        let mut file = new_private_file(&path)?;
        let copied = std::io::copy(&mut entry, &mut file)?;
        ensure!(copied == length, "Dolt archive payload is truncated");
        file.set_permissions(fs::Permissions::from_mode(if output == "dolt" {
            0o500
        } else {
            0o400
        }))?;
        file.sync_all()?;
        verify_payload(&path, length, checksum, output == "dolt")?;
    }
    ensure!(
        seen.len() == 4
            && [directory, bin_directory, executable, licenses]
                .iter()
                .all(|name| seen.contains(name.as_bytes())),
        "Dolt archive must contain exactly its four pinned entries"
    );
    let position = archive.into_inner().position() as usize;
    ensure!(
        expanded[position..].iter().all(|byte| *byte == 0),
        "Dolt archive contains trailing tar data"
    );
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn activate(candidate: &Path, destination: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(destination).is_err(),
        "Dolt cache destination appeared during installation"
    );
    fs::rename(candidate, destination).context("activate verified Dolt runtime")?;
    File::open(
        destination
            .parent()
            .context("Dolt cache requires a parent")?,
    )?
    .sync_all()?;
    Ok(())
}

async fn cache_lock(directory: &Path, timeout: Duration) -> Result<File> {
    let path = directory.join(".install.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&path)
        .context("open stable Dolt installation lock")?;
    checked_regular(&path, false)?;
    let start = tokio::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                ensure!(
                    start.elapsed() < timeout,
                    "timed out waiting for another Dolt installation; the held lock was preserved"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error).context("lock Dolt installation"),
        }
    }
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(&path)?;
    ensure!(
        (opened.dev(), opened.ino()) == (named.dev(), named.ino()),
        "Dolt installation lock was replaced while waiting"
    );
    Ok(file)
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => check_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
            check_directory(path)
        }
        Err(error) => Err(error).context("inspect private Dolt directory"),
    }
}

fn check_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Dolt directory must be an ordinary directory: {}",
        path.display()
    );
    ensure!(
        metadata.uid() == nix::unistd::getuid().as_raw(),
        "Dolt directory belongs to another user: {}",
        path.display()
    );
    ensure!(
        metadata.permissions().mode() & 0o077 == 0,
        "Dolt directory must be private (mode 0700): {}",
        path.display()
    );
    Ok(())
}

fn checked_regular(path: &Path, executable: bool) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect Dolt file {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1,
        "Dolt file must be a regular file without links: {}",
        path.display()
    );
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "configured Dolt file is not executable"
    );
    Ok(metadata)
}

fn open_regular(path: &Path) -> Result<File> {
    let expected = checked_regular(path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)?;
    let actual = file.metadata()?;
    ensure!(
        (expected.dev(), expected.ino()) == (actual.dev(), actual.ino()),
        "Dolt file changed while opening"
    );
    Ok(file)
}

fn new_private_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)?)
}

fn verify_payload(path: &Path, size: u64, expected: &str, executable: bool) -> Result<()> {
    let metadata = checked_regular(path, executable)?;
    ensure!(
        metadata.len() == size,
        "Dolt payload size mismatch: {}",
        path.display()
    );
    let mut file = open_regular(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    ensure!(
        hex_digest(&digest.finalize()) == expected,
        "Dolt payload checksum mismatch: {}",
        path.display()
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Prepare only this owned home; never load the invoking user's Dolt settings.
pub(crate) fn prepare_private_home(home: &Path) -> Result<()> {
    private_directory(home)?;
    for relative in ["home", "tmp", "root", "root/.dolt"] {
        private_directory(&home.join(relative))?;
    }
    let config = home.join("root/.dolt/config_global.json");
    let mut values = if config.try_exists()? {
        let file = open_regular(&config)?;
        ensure!(
            file.metadata()?.len() <= 64 * 1024,
            "private Dolt config is oversized"
        );
        serde_json::from_reader::<_, serde_json::Map<String, serde_json::Value>>(file)
            .context("private Dolt config must be a JSON object")?
    } else {
        ensure!(
            fs::symlink_metadata(&config).is_err(),
            "private Dolt config must not be a dangling link"
        );
        serde_json::Map::new()
    };
    for key in ["metrics.disabled", "versioncheck.disabled"] {
        values.insert(key.to_string(), "true".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(config.parent().unwrap())?;
    serde_json::to_writer(&mut temporary, &values)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(&config).map_err(|error| error.error)?;
    File::open(config.parent().unwrap())?.sync_all()?;
    Ok(())
}

pub(crate) fn isolated_command(binary: &Path, home: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.join("home"))
        .env("DOLT_ROOT_PATH", home.join("root"))
        .env("TMPDIR", home.join("tmp"))
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .current_dir(home)
        .kill_on_drop(true);
    command
}

/// Execute a finite, isolated exact-version probe after configuring its private
/// home to suppress upstream version checks and metrics child processes.
pub async fn verify_version(binary: &Path, private_home: &Path) -> Result<()> {
    verify_version_with_timeout(binary, private_home, VERSION_TIMEOUT).await
}

async fn verify_version_with_timeout(binary: &Path, home: &Path, timeout: Duration) -> Result<()> {
    checked_regular(binary, true)?;
    prepare_private_home(home)?;
    let mut child = isolated_command(binary, home)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start configured Dolt executable")?;
    let stdout = child
        .stdout
        .take()
        .context("Dolt version stdout is missing")?;
    let stderr = child
        .stderr
        .take()
        .context("Dolt version stderr is missing")?;
    let result = tokio::time::timeout(timeout, async {
        let (status, stdout, _stderr) = tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            bounded_output(stdout),
            bounded_output(stderr),
        )?;
        ensure!(status.success(), "Dolt version probe failed");
        ensure!(
            std::str::from_utf8(&stdout)?.trim() == format!("dolt version {DOLT_VERSION}"),
            "Kuru requires full Dolt {DOLT_VERSION}; configured executable reports another version"
        );
        Ok(())
    })
    .await;
    let result = result.unwrap_or_else(|_| Err(anyhow::anyhow!("Dolt version probe timed out")));
    if result.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

async fn bounded_output(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() as u64 <= OUTPUT_LIMIT,
        "Dolt version output exceeded its limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests;
