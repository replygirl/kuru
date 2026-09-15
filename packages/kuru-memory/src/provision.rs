//! Bounded, private installation of the exact supported full-Dolt engine.

use crate::MemoryOpenStage;
pub use crate::catalog::DOLT_VERSION;
use crate::catalog::{Asset, BUNDLED_ASSET, EMBEDDED_ARCHIVE, MAX_COMPRESSED, MAX_EXPANDED};
use crate::files::{self, PrivateTemp};
use crate::progress::ProgressReporter;
use anyhow::{Context, Result, bail, ensure};
use flate2::bufread::GzDecoder;
use kuru_core::MemoryConfig;
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info, seal_private};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    borrow::Cow,
    collections::HashSet,
    fs::{self, File, TryLockError},
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};
#[cfg(unix)]
use tokio::process::Command;

const LOCK_TIMEOUT: Duration = Duration::from_secs(180);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: u64 = 4096;
#[cfg(windows)]
const ACTIVATION_RETRY_LIMIT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const ACTIVATION_RETRY_SPACING: Duration = Duration::from_millis(20);

/// Extract the bundled engine or reuse its verified cache, including offline
/// first use. An explicit development binary still passes the exact version
/// guard; managed entries also pass their immutable payload checksums.
pub async fn provision(config: &MemoryConfig, default_cache: &Path) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_observed(config, default_cache, &mut progress).await
}

pub(crate) async fn provision_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    config.validate()?;
    if let Some(binary) = &config.dolt_binary {
        checked_regular(binary, true)?;
        let binary = binary
            .canonicalize()
            .context("resolve configured Dolt executable")?;
        progress.report(MemoryOpenStage::CheckingRuntimeVersion);
        private_probe(binary.clone()).await?;
        return Ok(binary);
    }
    provision_managed_observed(
        config,
        default_cache,
        BUNDLED_ASSET,
        Cow::Borrowed(EMBEDDED_ARCHIVE),
        progress,
    )
    .await
}

#[cfg(test)]
async fn provision_managed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_managed_observed(config, default_cache, asset, archive, &mut progress).await
}

async fn provision_managed_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    provision_with_extractor_observed(config, default_cache, asset, archive, extract, progress)
        .await
}

#[cfg(test)]
async fn provision_with_extractor(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    extractor: impl FnOnce(&[u8], &Path, Asset<'static>) -> Result<()> + Send + 'static,
) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    provision_with_extractor_observed(
        config,
        default_cache,
        asset,
        archive,
        extractor,
        &mut progress,
    )
    .await
}

async fn provision_with_extractor_observed(
    config: &MemoryConfig,
    default_cache: &Path,
    asset: Asset<'static>,
    archive: Cow<'static, [u8]>,
    extractor: impl FnOnce(&[u8], &Path, Asset<'static>) -> Result<()> + Send + 'static,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    let cache = config.cache_dir.as_deref().unwrap_or(default_cache);
    private_directory(cache)?;
    let cache = cache.canonicalize()?;
    let versions = cache.join(DOLT_VERSION);
    private_directory(&versions)?;
    let destination = versions.join(asset.target);
    if destination_exists(&destination)? {
        progress.report(MemoryOpenStage::VerifyingRuntimeCache);
        return verify_existing_cache(&destination, asset, progress).await;
    }
    progress.report(MemoryOpenStage::WaitingForRuntimeCache);
    let lock = cache_lock(&cache, LOCK_TIMEOUT).await?;
    if destination_exists(&destination)? {
        drop(lock);
        progress.report(MemoryOpenStage::VerifyingRuntimeCache);
        return verify_existing_cache(&destination, asset, progress).await;
    }
    let staging = PrivateTemp::new(".install-", Some(&versions))?;
    progress.report(MemoryOpenStage::ExtractingEmbeddedRuntime);
    let candidate = staging.path().join("runtime");
    let candidate_path = candidate.clone();
    let (staging, lock, extraction) = tokio::task::spawn_blocking(move || {
        let result = extractor(&archive, &candidate_path, asset);
        // Keep the stage and stable lock until the worker exits, even if its
        // caller is cancelled. Drop the stage before releasing the lock.
        (staging, lock, result)
    })
    .await?;
    extraction?;
    progress.report(MemoryOpenStage::CheckingRuntimeVersion);
    let probe_home = staging.path().join("probe");
    let candidate_path = candidate.clone();
    let probe_path = probe_home.clone();
    let (probe, staging, lock) = tokio::task::spawn_blocking(move || {
        // On cancellation, the completed output drops in this field order:
        // checked probe, disposable stage, then installation authority.
        (
            prepare_cold_probe(&candidate_path, &probe_path, asset),
            staging,
            lock,
        )
    })
    .await?;
    let probe = match probe {
        Ok(probe) => probe,
        Err(error) => {
            drop(staging);
            drop(lock);
            return Err(error);
        }
    };
    let (probe, (staging, lock)) = probe.probe((staging, lock)).await?;
    activate_staged_after_probe(staging, lock, probe, &candidate, &destination).await?;
    Ok(destination.join(asset.executable_name))
}

fn destination_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn verify_existing_cache(
    destination: &Path,
    asset: Asset<'_>,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    verified_cache_observed(destination, asset, progress)
        .await
        .with_context(|| {
            format!(
                "Dolt cache is invalid at {}; preserve or remove that version directory and retry",
                destination.display()
            )
        })
}

#[cfg(test)]
async fn verified_cache(directory: &Path, asset: Asset<'_>) -> Result<PathBuf> {
    let mut progress = ProgressReporter::silent();
    verified_cache_observed(directory, asset, &mut progress).await
}

async fn verified_cache_observed(
    directory: &Path,
    asset: Asset<'_>,
    progress: &mut ProgressReporter,
) -> Result<PathBuf> {
    let binary = directory.join(asset.executable_name);
    let directory_path = directory.to_owned();
    let executable_name = asset.executable_name.to_owned();
    let executable_bytes = asset.executable_bytes;
    let executable_sha256 = asset.executable_sha256.to_owned();
    let license_bytes = asset.license_bytes;
    let license_sha256 = asset.license_sha256.to_owned();
    let checked = tokio::task::spawn_blocking(move || {
        CheckedCache::open_and_verify(
            &directory_path,
            &executable_name,
            executable_bytes,
            &executable_sha256,
            license_bytes,
            &license_sha256,
        )
    })
    .await??;
    progress.report(MemoryOpenStage::CheckingRuntimeVersion);
    checked.probe(binary.clone()).await?;
    Ok(binary)
}

struct CheckedCache {
    directory: Directory,
    executable_name: std::ffi::OsString,
    executable: File,
    licenses: File,
}

impl CheckedCache {
    fn open_and_verify(
        path: &Path,
        executable_name: &str,
        executable_bytes: u64,
        executable_sha256: &str,
        license_bytes: u64,
        license_sha256: &str,
    ) -> Result<Self> {
        let directory = files::directory(path)?;
        let executable_name = std::ffi::OsString::from(executable_name);
        let mut executable = directory.read(&executable_name)?;
        let mut licenses = directory.read(std::ffi::OsStr::new("LICENSES"))?;
        verify_payload_file(
            &mut executable,
            executable_bytes,
            executable_sha256,
            true,
            &path.join(&executable_name),
        )?;
        verify_payload_file(
            &mut licenses,
            license_bytes,
            license_sha256,
            false,
            &path.join("LICENSES"),
        )?;
        let checked = Self {
            directory,
            executable_name,
            executable,
            licenses,
        };
        checked.revalidate()?;
        Ok(checked)
    }

    fn revalidate(&self) -> Result<()> {
        self.directory.revalidate()?;
        self.directory
            .verify(&self.executable_name, &self.executable)?;
        self.directory
            .verify(std::ffi::OsStr::new("LICENSES"), &self.licenses)?;
        Ok(())
    }

    async fn probe(self, binary: PathBuf) -> Result<()> {
        let home = PrivateTemp::new("kuru-dolt-version-", None)?;
        let home_path = home.path().to_owned();
        let (send, receive) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("kuru-dolt-probe".into())
            .spawn(move || {
                // Keep the exact verified payload handles and private probe home
                // until the owned child has been reaped, even if the caller exits.
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create owned Dolt probe executor")
                    .and_then(|runtime| {
                        self.revalidate()
                            .and_then(|()| runtime.block_on(verify_version(&binary, &home_path)))
                    });
                let _ = send.send((self, home, result));
            })
            .context("start owned Dolt probe thread")?;
        let (_checked, _home, result) = receive
            .await
            .context("owned Dolt probe did not return its result")?;
        result
    }
}

async fn private_probe(binary: PathBuf) -> Result<()> {
    let home = PrivateTemp::new("kuru-dolt-version-", None)?;
    owned_probe(binary, home.path().to_owned(), home).await?;
    Ok(())
}

async fn owned_probe<T: Send + 'static>(binary: PathBuf, home: PathBuf, retained: T) -> Result<T> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("kuru-dolt-probe".into())
        .spawn(move || {
            // This process owner must survive destruction of its caller's executor.
            // Sending to a canceled caller drops retained resources here, only
            // after the bounded probe and actual owned child reap have completed.
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("create owned Dolt probe executor")
                .and_then(|runtime| runtime.block_on(verify_version(&binary, &home)));
            let _ = send.send((retained, result));
        })
        .context("start owned Dolt probe thread")?;
    let (retained, result) = receive
        .await
        .context("owned Dolt probe did not return its result")?;
    result?;
    Ok(retained)
}

#[derive(Debug)]
struct CheckedColdProbe {
    directory: Directory,
    executable_name: std::ffi::OsString,
    executable: File,
    binary: PathBuf,
    home: PathBuf,
}

impl CheckedColdProbe {
    fn revalidate(&self) -> Result<()> {
        self.directory.revalidate()?;
        self.directory
            .verify(&self.executable_name, &self.executable)?;
        Ok(())
    }

    async fn probe<T: Send + 'static>(self, retained: T) -> Result<(Self, T)> {
        let binary = self.binary.clone();
        let home = self.home.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("kuru-dolt-cold-probe".into())
            .spawn(move || {
                // Keep the checked copy, stage and installation authority until
                // the owned process is reaped, including after caller cancellation.
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create owned Dolt cold probe executor")
                    .and_then(|runtime| {
                        self.revalidate()
                            .and_then(|()| runtime.block_on(verify_version(&binary, &home)))
                    });
                let _ = send.send((self, retained, result));
            })
            .context("start owned Dolt cold probe thread")?;
        let (probe, retained, result) = receive
            .await
            .context("owned Dolt cold probe did not return its result")?;
        if let Err(error) = result {
            // Drop the probe handles and stage before releasing the retained lock.
            drop(probe);
            drop(retained);
            return Err(error);
        }
        Ok((probe, retained))
    }
}

fn prepare_cold_probe(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
) -> Result<CheckedColdProbe> {
    prepare_cold_probe_with(candidate, probe_home, asset, |_| Ok(()))
}

#[cfg(test)]
fn prepare_cold_probe_observed(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
    observer: impl FnOnce(&mut File) -> Result<()>,
) -> Result<CheckedColdProbe> {
    prepare_cold_probe_with(candidate, probe_home, asset, observer)
}

fn prepare_cold_probe_with(
    candidate: &Path,
    probe_home: &Path,
    asset: Asset<'_>,
    observer: impl FnOnce(&mut File) -> Result<()>,
) -> Result<CheckedColdProbe> {
    private_directory(probe_home)?;
    let source_directory = files::directory(candidate)?;
    let executable_name = std::ffi::OsStr::new(asset.executable_name);
    let source_path = candidate.join(executable_name);
    let mut source = source_directory.read(executable_name)?;
    verify_payload_file(
        &mut source,
        asset.executable_bytes,
        asset.executable_sha256,
        true,
        &source_path,
    )?;
    source_directory.verify(executable_name, &source)?;
    source.rewind()?;

    let probe_directory = files::directory(probe_home)?;
    let probe_path = probe_home.join(executable_name);
    let mut probe_writer = probe_directory.create_new(executable_name)?;
    ensure!(
        std::io::copy(
            &mut (&mut source).take(asset.executable_bytes + 1),
            &mut probe_writer,
        )? == asset.executable_bytes,
        "Dolt cold probe copy size mismatch"
    );
    seal_private(&probe_writer, true)?;
    probe_writer.sync_all()?;
    observer(&mut probe_writer)?;
    probe_writer.sync_all()?;
    drop(probe_writer);

    // Retain only read authority after the test seam and before execution.
    let mut probe = probe_directory.read(executable_name)?;
    probe.rewind()?;
    verify_payload_file(
        &mut probe,
        asset.executable_bytes,
        asset.executable_sha256,
        true,
        &probe_path,
    )?;
    source_directory.verify(executable_name, &source)?;
    probe_directory.verify(executable_name, &probe)?;
    ensure!(
        regular_file_info(&source)?.identity != regular_file_info(&probe)?.identity,
        "Dolt cold probe copy unexpectedly retained the source identity"
    );
    Ok(CheckedColdProbe {
        directory: probe_directory,
        executable_name: executable_name.to_owned(),
        executable: probe,
        binary: probe_path,
        home: probe_home.to_owned(),
    })
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
    if asset.format == "zip" {
        return extract_zip(archive, destination, asset);
    }
    ensure!(
        asset.format == "tar.gz",
        "unsupported pinned Dolt archive format"
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
        seal_private(&file, output == "dolt")?;
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
    #[cfg(unix)]
    File::open(destination)?.sync_all()?;
    Ok(())
}

struct StagedActivation {
    // Rust drops fields in declaration order. Close the candidate and probe
    // authorities before the disposable private stage, then the cache lease.
    source: Option<Directory>,
    probe: Option<CheckedColdProbe>,
    staging: PrivateTemp,
    lock: CacheLock,
}

impl StagedActivation {
    fn retain(self, error: anyhow::Error) -> anyhow::Error {
        let Self {
            source,
            probe,
            staging,
            lock,
        } = self;
        // Close the checked candidate before retaining its stage, then release
        // the cache lock only after stage ownership has been decided.
        drop(source);
        drop(probe);
        let retained = staging.keep();
        drop(lock);
        error.context(format!(
            "verified Dolt activation failed; preserved private stage at {}",
            retained.display()
        ))
    }
}

#[cfg(test)]
async fn activate_staged(
    staging: PrivateTemp,
    lock: CacheLock,
    candidate: &Path,
    destination: &Path,
) -> Result<()> {
    activate_staged_with(staging, lock, None, candidate, destination, |_| {}).await
}

async fn activate_staged_after_probe(
    staging: PrivateTemp,
    lock: CacheLock,
    probe: CheckedColdProbe,
    candidate: &Path,
    destination: &Path,
) -> Result<()> {
    activate_staged_with(staging, lock, Some(probe), candidate, destination, |_| {}).await
}

#[cfg(test)]
async fn activate_staged_observed(
    staging: PrivateTemp,
    lock: CacheLock,
    candidate: &Path,
    destination: &Path,
    observer: impl FnMut(bool),
) -> Result<()> {
    activate_staged_with(staging, lock, None, candidate, destination, observer).await
}

async fn activate_staged_with(
    staging: PrivateTemp,
    lock: CacheLock,
    probe: Option<CheckedColdProbe>,
    candidate: &Path,
    destination: &Path,
    mut observer: impl FnMut(bool),
) -> Result<()> {
    let mut activation = StagedActivation {
        source: None,
        probe,
        staging,
        lock,
    };
    activation.source = match files::directory(candidate) {
        Ok(source) => Some(source),
        Err(error) => {
            return Err(activation.retain(error.context("open verified Dolt activation source")));
        }
    };
    let source = activation
        .source
        .as_ref()
        .expect("the activation source is set before any move attempt");

    #[cfg(not(windows))]
    {
        match activate_once(source, destination) {
            Ok(files::DirectoryMove::Moved(_)) => {
                observer(false);
                Ok(())
            }
            Ok(files::DirectoryMove::ProvenNoMove(error)) => {
                observer(true);
                Err(activation.retain(error))
            }
            Err(error) => Err(activation.retain(error)),
        }
    }

    #[cfg(windows)]
    {
        let deadline = tokio::time::Instant::now() + ACTIVATION_RETRY_LIMIT;
        let mut first_error = None;
        let mut retries = 0_u32;
        loop {
            if first_error.is_some() {
                if tokio::time::Instant::now() >= deadline {
                    let error = first_error
                        .expect("a pending activation retry retains its first checked error");
                    return Err(activation.retain(expired_activation_error(error, retries)));
                }
                retries += 1;
            }
            match activate_once(source, destination) {
                Ok(files::DirectoryMove::Moved(_)) => {
                    observer(false);
                    return Ok(());
                }
                Ok(files::DirectoryMove::ProvenNoMove(error)) => {
                    observer(true);
                    if error
                        .downcast_ref::<kuru_platform::fs::PublicationError>()
                        .is_none_or(|publication| publication.error().raw_os_error() != Some(5))
                    {
                        let error = terminal_activation_error(first_error, error, retries);
                        return Err(activation.retain(error));
                    }
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        let error = terminal_activation_error(first_error, error, retries);
                        return Err(activation.retain(error));
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    tokio::time::sleep((deadline - now).min(ACTIVATION_RETRY_SPACING)).await;
                }
                Err(error) => {
                    let error = terminal_activation_error(first_error, error, retries);
                    return Err(activation.retain(error));
                }
            }
        }
    }
}

#[cfg(windows)]
fn terminal_activation_error(
    first_error: Option<anyhow::Error>,
    error: anyhow::Error,
    retries: u32,
) -> anyhow::Error {
    match first_error {
        Some(first_error) => first_error.context(format!(
            "runtime activation recovery stopped after {retries} retries; latest checked result: {error:#}"
        )),
        None => error,
    }
}

#[cfg(windows)]
fn expired_activation_error(error: anyhow::Error, retries: u32) -> anyhow::Error {
    error.context(format!(
        "runtime activation recovery stopped after {retries} retries before starting another native move"
    ))
}

#[cfg(test)]
fn activate(candidate: &Path, destination: &Path) -> Result<()> {
    let source = files::directory(candidate)?;
    match activate_once(&source, destination)? {
        files::DirectoryMove::Moved(_) => Ok(()),
        files::DirectoryMove::ProvenNoMove(error) => Err(error),
    }
}

fn activate_once(source: &Directory, destination: &Path) -> Result<files::DirectoryMove> {
    ensure!(
        fs::symlink_metadata(destination).is_err(),
        "Dolt cache destination appeared during installation"
    );
    match files::move_directory_checked(source, destination) {
        Ok(files::DirectoryMove::Moved(directory)) => Ok(files::DirectoryMove::Moved(directory)),
        Ok(files::DirectoryMove::ProvenNoMove(error)) => Ok(files::DirectoryMove::ProvenNoMove(
            error.context("activate verified Dolt runtime"),
        )),
        Err(error) => Err(error.context("activate verified Dolt runtime")),
    }
}

fn extract_zip(bytes: &[u8], destination: &Path, asset: Asset<'_>) -> Result<()> {
    use kuru_archive::zip::{Archive, Limits, MemberKind, MemberSpec};
    let directory = format!("{}/", asset.stem);
    let bin_directory = format!("{}/bin/", asset.stem);
    let executable = format!("{}/bin/{}", asset.stem, asset.executable_name);
    let licenses = format!("{}/LICENSES", asset.stem);
    let expected = [
        MemberSpec {
            name: &directory,
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
        MemberSpec {
            name: &bin_directory,
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
        MemberSpec {
            name: &executable,
            kind: MemberKind::File,
            max_bytes: asset.executable_bytes,
            exact_bytes: Some(asset.executable_bytes),
            unix_mode: Some(0o100755),
        },
        MemberSpec {
            name: &licenses,
            kind: MemberKind::File,
            max_bytes: asset.license_bytes,
            exact_bytes: Some(asset.license_bytes),
            unix_mode: Some(0o100644),
        },
    ];
    let mut archive = Archive::open(
        bytes,
        &expected,
        Limits {
            max_compressed_bytes: MAX_COMPRESSED,
            max_expanded_bytes: asset.expanded_bytes,
            allow_ntfs_timestamps: true,
        },
    )?;
    private_directory(destination)?;
    let parent = files::directory(destination)?;
    for (member, output, size, digest, executable) in [
        (
            &executable,
            asset.executable_name,
            asset.executable_bytes,
            asset.executable_sha256,
            true,
        ),
        (
            &licenses,
            "LICENSES",
            asset.license_bytes,
            asset.license_sha256,
            false,
        ),
    ] {
        let mut file = parent.create_new(std::ffi::OsStr::new(output))?;
        ensure!(
            archive.copy(member, &mut file)? == size,
            "Dolt ZIP payload size mismatch"
        );
        file.sync_all()?;
        seal_private(&file, executable)?;
        parent.verify(std::ffi::OsStr::new(output), &file)?;
        drop(file);
        verify_payload(&destination.join(output), size, digest, executable)?;
    }
    #[cfg(unix)]
    File::open(destination)?.sync_all()?;
    Ok(())
}

struct CacheLock {
    file: File,
    _directory: Directory,
}
impl std::ops::Deref for CacheLock {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}

async fn cache_lock(directory: &Path, timeout: Duration) -> Result<CacheLock> {
    let path = directory.join(".install.lock");
    let directory = Directory::open(directory, Privacy::OwnerOnly, NameRetention::Pinned)?;
    let file = directory
        .lock_file(files::name(&path)?)
        .context("open stable Dolt installation lock")?;
    checked_regular(&path, false)?;
    let start = tokio::time::Instant::now();
    loop {
        directory.verify(files::name(&path)?, &file)?;
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
    directory
        .verify(files::name(&path)?, &file)
        .context("Dolt installation lock was replaced while waiting")?;
    Ok(CacheLock {
        file,
        _directory: directory,
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    files::private_dir(path).context("inspect private Dolt directory")
}

fn checked_regular(path: &Path, executable: bool) -> Result<fs::Metadata> {
    let (_parent, file) = files::read(path, Privacy::Inherited)
        .with_context(|| format!("inspect Dolt file {}", path.display()))?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "configured Dolt file is not executable"
    );
    #[cfg(windows)]
    let _ = executable;
    Ok(metadata)
}

fn open_regular(path: &Path) -> Result<File> {
    let (_parent, file) = files::read(path, Privacy::Inherited)?;
    Ok(file)
}

fn new_private_file(path: &Path) -> Result<File> {
    let parent = files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    Ok(parent.create_new(files::name(path)?)?)
}

fn verify_payload(path: &Path, size: u64, expected: &str, executable: bool) -> Result<()> {
    let (parent, mut file) = files::read(path, Privacy::Inherited)
        .with_context(|| format!("inspect Dolt file {}", path.display()))?;
    verify_payload_file(&mut file, size, expected, executable, path)?;
    parent.verify(files::name(path)?, &file)?;
    Ok(())
}

fn verify_payload_file(
    file: &mut File,
    size: u64,
    expected: &str,
    executable: bool,
    path: &Path,
) -> Result<()> {
    let metadata = file.metadata()?;
    #[cfg(unix)]
    ensure!(
        !executable || metadata.permissions().mode() & 0o111 != 0,
        "configured Dolt file is not executable"
    );
    #[cfg(windows)]
    let _ = executable;
    ensure!(
        metadata.len() == size,
        "Dolt payload size mismatch: {}",
        path.display()
    );
    kuru_platform::fs::require_private(file)?;
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
    files::write_dolt_config(&config, &serde_json::to_vec(&values)?)?;
    Ok(())
}

#[cfg(unix)]
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
    let mut child = crate::engine::spawn(
        binary,
        home,
        home,
        vec!["version".into()],
        Vec::new(),
        false,
    )
    .await
    .context("start configured Dolt executable")?;
    let stdout = child.stdout().context("Dolt version stdout is missing")?;
    let stderr = child.stderr().context("Dolt version stderr is missing")?;
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
        let _ = child.kill();
        if child.wait().await.is_err() {
            eprintln!(
                "Dolt version cleanup is delayed; retaining the owned process and probe resources"
            );
            loop {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
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
mod native_tests;
#[cfg(all(test, unix))]
mod tests;
