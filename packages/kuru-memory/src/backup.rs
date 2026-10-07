//! Checked metadata for one native Dolt dataset-root backup.
//!
//! This module reads only the pinned NBS v5 manifest header. Dolt owns its
//! graph and chunk codec; an independent restore must validate that graph.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

mod capture;
mod native;

const FORMAT: u32 = 1;
const NBS_HEADER_BYTES: u64 = 256;
const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 65_536;
const MAX_NAME_BYTES: usize = 128;
const MAX_RELATIVE_BYTES: usize = 1024;
const MAX_DIRECTORY_DEPTH: usize = 8;
const BUFFER_BYTES: usize = 64 * 1024;
pub(crate) const SQL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4 * 60 * 60);

/// Cancellation of one backup, verification or restore operation. Requesting
/// cancellation does not establish completion: await the same operation so
/// its SQL session, native children and retained stage can actually settle.
#[derive(Clone, Debug, Default)]
pub struct BackupCancellation(Arc<Cancellation>);

#[derive(Debug, Default)]
struct Cancellation {
    requested: AtomicBool,
    wake: tokio::sync::Notify,
}

impl BackupCancellation {
    pub fn cancel(&self) {
        self.0.requested.store(true, Ordering::Release);
        self.0.wake.notify_waiters();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.requested.load(Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let wake = self.0.wake.notified();
            if self.is_cancelled() {
                return;
            }
            wake.await;
        }
    }
}

/// Safe completion metadata; the full private manifest is not command output.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupResult {
    pub dataset_root: String,
    pub schema_version: i32,
    pub refs: u64,
}

/// Restore completion separates the original immutable image root from the
/// target main revision after snapshot preparation and released migrations.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreResult {
    pub source_dataset_root: String,
    pub schema_version: i32,
    pub revision: String,
}

impl BackupResult {
    pub(crate) fn from_manifest(manifest: &BackupManifest) -> Result<Self> {
        Ok(Self {
            dataset_root: manifest.dataset_root.clone(),
            schema_version: manifest.schema_version,
            refs: u64::try_from(manifest.refs.len())
                .context("backup ref count exceeds integer range")?,
        })
    }
}

/// Retain the selected image's directory identity throughout independent
/// native validation. This proves only the physical record until that work
/// has completed; it is never a public restorability result by itself.
pub(crate) struct CheckedBackup {
    directory: Directory,
    pub(crate) manifest: BackupManifest,
    pub(crate) manifest_sha256: String,
}

impl CheckedBackup {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let directory = crate::files::directory(path)?;
        let mut names = Vec::new();
        for entry in fs::read_dir(directory.path())? {
            names.push(entry?.file_name());
            ensure!(names.len() <= 2, "backup root has an unexpected entry");
        }
        names.sort_unstable();
        ensure!(
            names
                == [
                    std::ffi::OsString::from("backup.json"),
                    std::ffi::OsString::from("image")
                ],
            "backup root has missing or unexpected entries"
        );
        let path = directory.path().join("backup.json");
        let bytes = crate::files::read_bytes(&path, MAX_METADATA_BYTES)?;
        let manifest: BackupManifest =
            serde_json::from_slice(&bytes).context("decode Kuru backup manifest")?;
        manifest.verify_image(&directory.path().join("image"))?;
        directory.revalidate()?;
        let manifest_sha256 = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            directory,
            manifest,
            manifest_sha256,
        })
    }

    pub(crate) fn image_path(&self) -> std::path::PathBuf {
        self.directory.path().join("image")
    }

    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        self.directory.revalidate()?;
        let found = Self::read(self.directory.path())?;
        ensure!(
            found.directory.identity() == self.directory.identity()
                && found.manifest_sha256 == self.manifest_sha256
                && found.manifest == self.manifest,
            "selected backup changed during independent native validation"
        );
        Ok(())
    }
}

/// A fully validated unpublished image. Publication is a single synchronous
/// absent-directory move; a lost reply never authorizes resending it.
pub(crate) struct PreparedBackup {
    directory: Directory,
    target: std::path::PathBuf,
    manifest: BackupManifest,
}

impl PreparedBackup {
    pub(crate) fn publish(self, cancellation: &BackupCancellation) -> Result<BackupResult> {
        let result = BackupResult::from_manifest(&self.manifest)?;
        self.directory.revalidate()?;
        // No await separates the final cancellation/peer-EOF observation
        // from activation. Once the move succeeds, report that actual outcome
        // even if cancellation is requested before its reply is delivered.
        ensure!(
            !cancellation.is_cancelled(),
            "backup cancelled before publication; validated private image retained at {:?}",
            self.directory.path()
        );
        match crate::files::move_directory_checked(&self.directory, &self.target)? {
            crate::files::DirectoryMove::Moved(_) => Ok(result),
            crate::files::DirectoryMove::ProvenNoMove(error) => Err(error).with_context(|| {
                format!(
                    "validated private backup remains at {:?}",
                    self.directory.path()
                )
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_path(&self) -> &Path {
        self.directory.path()
    }
}

pub(crate) async fn prepare(
    store: &crate::store::MemoryStore,
    project: &Path,
    target: &Path,
    cancellation: &BackupCancellation,
) -> Result<PreparedBackup> {
    let source = store.backup_source(project)?;
    prepare_from_owner(store, source, target, cancellation).await
}

pub(crate) async fn prepare_from_owner(
    store: &crate::store::MemoryStore,
    source: BackupSource,
    target: &Path,
    cancellation: &BackupCancellation,
) -> Result<PreparedBackup> {
    ensure!(target.is_absolute(), "backup destination must be absolute");
    let parent = crate::files::parent(target, Privacy::Inherited, NameRetention::Movable)?;
    let live = crate::files::directory(&store.status().await?.directory)?;
    ensure!(
        !parent.is_within(&live)?,
        "backup destination cannot be inside the live memory directory"
    );
    ensure!(
        matches!(fs::symlink_metadata(target), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "backup destination already exists or cannot be checked"
    );
    ensure!(
        !cancellation.is_cancelled(),
        "backup cancelled before staging"
    );
    let name = format!("kuru-backup-stage-{}", uuid::Uuid::new_v4());
    let directory = parent.create_private_directory(OsStr::new(&name))?;
    let image = directory.create_private_directory(OsStr::new("image"))?;
    let path = directory.path().to_owned();
    let prepared = async {
        // Any uncertain SQL result exits before image hashing or validation.
        // No Drop removes this named destination; the source's existing
        // supervisor owns any remaining native work through actual reap.
        capture::copy(
            store.backup_connection()?,
            image.path().to_owned(),
            cancellation,
        )
        .await?;
        directory.revalidate()?;
        image.revalidate()?;
        let inspected = inspect_image(
            store.image_runtime(),
            &parent,
            image.path(),
            source.clone(),
            cancellation,
        )
        .await?;
        let manifest = BackupManifest::new(
            source,
            inspected.schema_version,
            inspected.refs,
            image.path(),
        )?;
        manifest.write(&directory.path().join("backup.json"))?;
        manifest.verify_image(image.path())?;
        ensure!(
            BackupManifest::read(&directory.path().join("backup.json"))? == manifest,
            "completed backup metadata changed before publication"
        );
        ensure!(
            !cancellation.is_cancelled(),
            "backup cancelled before publication"
        );
        directory.revalidate()?;
        Ok(PreparedBackup {
            directory,
            target: target.to_owned(),
            manifest,
        })
    }
    .await;
    prepared.with_context(|| format!("unpublished private backup retained at {path:?}"))
}

async fn inspect_image(
    runtime: ImageRuntime,
    parent: &Directory,
    image: &Path,
    source: BackupSource,
    cancellation: &BackupCancellation,
) -> Result<crate::store::backup_validation::Inspection> {
    let name = format!("kuru-backup-validation-{}", uuid::Uuid::new_v4());
    let directory = parent.create_private_directory(OsStr::new(&name))?;
    let path = directory.path().to_owned();
    let stage = native::ImageStage::new(directory, runtime.binary, runtime.lifecycle_root, None)?;
    let outcome = async {
        stage.restore(image.to_owned(), cancellation).await?;
        stage.fsck(cancellation).await?;
        let inspected = stage
            .inspect(source, runtime.supervisor, cancellation)
            .await?;
        stage.discard().await?;
        Ok::<_, anyhow::Error>(inspected)
    }
    .await;
    outcome.with_context(|| format!("private native validation stage retained at {path:?}"))
}

/// Structural checks alone are not public verification: independently restore
/// the selected immutable image and prove native graph/ref/working-set reads.
pub(crate) async fn verify(
    options: &crate::store::OpenOptions,
    path: &Path,
    cancellation: &BackupCancellation,
) -> Result<BackupResult> {
    options.config.validate()?;
    let checked = CheckedBackup::read(path)?;
    ensure!(
        !cancellation.is_cancelled(),
        "backup verification cancelled before launch"
    );
    let runtime = image_runtime(options).await?;
    let parent =
        crate::files::ensure_private_directory(&options.data_dir.join("backup-validation"))?;
    let inspected = inspect_image(
        runtime,
        &parent,
        &checked.image_path(),
        BackupSource::from_manifest(&checked.manifest),
        cancellation,
    )
    .await?;
    ensure!(
        inspected.schema_version == checked.manifest.schema_version
            && inspected.refs == checked.manifest.refs,
        "native backup observations differ from its captured metadata"
    );
    checked.verify_unchanged()?;
    BackupResult::from_manifest(&checked.manifest)
}

pub(crate) async fn image_runtime(options: &crate::store::OpenOptions) -> Result<ImageRuntime> {
    #[cfg(unix)]
    let lifecycle_root = None;
    #[cfg(windows)]
    let lifecycle_root = Some(options.data_dir.join("memory/lifecycles"));
    Ok(ImageRuntime {
        binary: crate::provision::provision(&options.config, &options.data_dir.join("tools/dolt"))
            .await?,
        supervisor: match &options.supervisor {
            Some(executable) => executable.clone(),
            None => kuru_platform::running_executable()?,
        },
        lifecycle_root,
    })
}

pub(crate) async fn restore(
    options: crate::store::OpenOptions,
    project: &Path,
    path: &Path,
    remap: bool,
    cancellation: &BackupCancellation,
) -> Result<RestoreResult> {
    options.config.validate()?;
    ensure!(
        !options.read_only && options.expected_instance.is_none(),
        "restore requires an absent writable target, not existing-client recovery"
    );
    ensure!(
        crate::service::canonical_project_scope(project)? == options.project_scope,
        "restore target does not match the canonical project"
    );
    let checked = CheckedBackup::read(path)?;
    let project_path = crate::service::project_path_bytes(project);
    ensure!(
        remap || project_path == checked.manifest.source_project_path,
        "restoring to another canonical project requires --remap-project"
    );
    let target = crate::store::project_directory(&options.data_dir, &options.project_scope)?;
    ensure!(
        matches!(fs::symlink_metadata(&target), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "restore target already exists or cannot be checked"
    );
    ensure!(
        !cancellation.is_cancelled(),
        "restore cancelled before maintenance admission"
    );
    let maintenance = crate::service::acquire_maintenance_permit(&options).await?;
    // The one permit owns native maintenance, owner/startup coordination and
    // all stage work. No nested project-exclusive permit is acquired.
    let runtime = image_runtime(&options).await?;
    let parent = crate::files::ensure_private_directory(
        target.parent().context("restore target has no parent")?,
    )?;
    let name = format!(
        "{}.staging-{}",
        target
            .file_name()
            .context("restore target has no name")?
            .to_string_lossy(),
        uuid::Uuid::new_v4()
    );
    let directory = parent.create_private_directory(OsStr::new(&name))?;
    let retained_path = directory.path().to_owned();
    let stage = native::ImageStage::new(
        directory,
        runtime.binary,
        runtime.lifecycle_root.clone(),
        Some(maintenance),
    )?;
    let outcome = async {
        stage.restore(checked.image_path(), cancellation).await?;
        stage.directory.revalidate()?;
        ensure!(
            pinned_nbs_root(
                &stage.directory.path().join("data/kuru/.dolt/noms"),
                Privacy::Inherited
            )? == checked.manifest.dataset_root,
            "native restored dataset root differs from selected image"
        );
        stage.fsck(cancellation).await?;
        let mut source = BackupSource::from_manifest(&checked.manifest);
        // Mint only target-canonical external authority. Copied SQL/history
        // coordinates remain those of the selected immutable image.
        source.project_path = project_path.clone();
        source.storage_scope = options.project_scope.clone();
        let inspected = stage
            .inspect(source.clone(), runtime.supervisor.clone(), cancellation)
            .await?;
        ensure!(
            inspected.schema_version == checked.manifest.schema_version
                && inspected.refs == checked.manifest.refs,
            "native restored observations differ from selected image"
        );
        stage
            .prepare_restore(
                source,
                runtime.supervisor,
                cancellation,
                crate::store::backup_restore::Preparation {
                    manifest: checked.manifest.clone(),
                    manifest_sha256: checked.manifest_sha256.clone(),
                    project_path,
                    project_scope: options.project_scope.clone(),
                },
            )
            .await?;
        // Check the newly appended graph too; original validity was already
        // proved before any target-only snapshot or migration mutation.
        stage.fsck(cancellation).await?;
        let (schema_version, revision) =
            crate::store::backup_restore::ready(stage.directory.path(), &options.project_scope)?;
        let result = RestoreResult {
            source_dataset_root: checked.manifest.dataset_root.clone(),
            schema_version,
            revision,
        };
        checked.verify_unchanged()?;
        let mut seal = crate::server::Server::quiescence_at(
            stage.directory.path(),
            runtime.lifecycle_root.as_deref(),
            crate::server::close_budget(),
        )
        .await?;
        stage.directory.revalidate()?;
        ensure!(
            seal.directory.identity() == stage.directory.identity(),
            "restore stopped-stage seal belongs to another directory"
        );
        ensure!(
            !cancellation.is_cancelled(),
            "restore cancelled before target publication"
        );
        // Actual publication is synchronous while the exact stopped stage
        // and the single maintenance permit are still retained.
        seal.move_to(&target)?;
        drop(seal);
        Ok(result)
    }
    .await;
    outcome
        .with_context(|| format!("unactivated private restore stage retained at {retained_path:?}"))
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format: u32,
    pub dolt_version: String,
    pub schema_version: i32,
    pub source_project_path: Vec<u8>,
    pub source_storage_scope: String,
    pub history_scope: String,
    pub source_store_instance: String,
    pub sql_origin_instance: String,
    pub sql_origin_scope: String,
    pub dataset_root: String,
    pub refs: Vec<BackupRef>,
    pub directories: Vec<String>,
    pub files: Vec<BackupFile>,
}

/// Native observations from the independently restored immutable image.
/// These describe data; no ref or namespace is a live claim capability.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRef {
    pub kind: BackupRefKind,
    pub name: String,
    pub head: String,
    pub working: Option<BackupWorkingSet>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupRefKind {
    Branch,
    Tag,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupWorkingSet {
    pub working_root: String,
    pub staged_root: String,
    pub dirty: bool,
}

/// Source authority is supplied by the checked store, not the native image.
#[derive(Clone)]
pub(crate) struct BackupSource {
    pub project_path: Vec<u8>,
    pub storage_scope: String,
    pub history_scope: String,
    pub store_instance: String,
    pub sql_origin_instance: String,
    pub sql_origin_scope: String,
}

impl BackupSource {
    fn from_manifest(manifest: &BackupManifest) -> Self {
        Self {
            project_path: manifest.source_project_path.clone(),
            storage_scope: manifest.source_storage_scope.clone(),
            history_scope: manifest.history_scope.clone(),
            store_instance: manifest.source_store_instance.clone(),
            sql_origin_instance: manifest.sql_origin_instance.clone(),
            sql_origin_scope: manifest.sql_origin_scope.clone(),
        }
    }
}

/// The owning open's already selected native inputs. These are private
/// launch metadata, not serialized backup data or caller authority.
#[derive(Clone)]
pub(crate) struct ImageRuntime {
    pub binary: std::path::PathBuf,
    pub supervisor: std::path::PathBuf,
    pub lifecycle_root: Option<std::path::PathBuf>,
}

impl ImageRuntime {
    pub(crate) fn from_server(options: &crate::server::ServerOptions) -> Self {
        Self {
            binary: options.binary.clone(),
            supervisor: options.supervisor.clone(),
            lifecycle_root: options.lifecycle_root.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

impl BackupManifest {
    pub(crate) fn new(
        source: BackupSource,
        schema_version: i32,
        refs: Vec<BackupRef>,
        image: &Path,
    ) -> Result<Self> {
        let dataset_root = pinned_nbs_root(image, Privacy::OwnerOnly)?;
        let (directories, files) = inventory(image)?;
        let manifest = Self {
            format: FORMAT,
            dolt_version: crate::provision::DOLT_VERSION.into(),
            schema_version,
            source_project_path: source.project_path,
            source_storage_scope: source.storage_scope,
            history_scope: source.history_scope,
            source_store_instance: source.store_instance,
            sql_origin_instance: source.sql_origin_instance,
            sql_origin_scope: source.sql_origin_scope,
            dataset_root,
            refs,
            directories,
            files,
        };
        manifest.validate_fields()?;
        Ok(manifest)
    }

    pub(crate) fn read(path: &Path) -> Result<Self> {
        let bytes = crate::files::read_bytes(path, MAX_METADATA_BYTES)?;
        let manifest: Self =
            serde_json::from_slice(&bytes).context("decode Kuru backup manifest")?;
        manifest.validate_fields()?;
        Ok(manifest)
    }

    pub(crate) fn write(&self, path: &Path) -> Result<()> {
        self.validate_fields()?;
        let mut encoded = MetadataBuffer(Vec::new());
        serde_json::to_writer(&mut encoded, self).context("encode bounded Kuru backup metadata")?;
        // This record is written only inside an unpublished private backup.
        // The enclosing checked directory move publishes it atomically with
        // its image; generic replace-record staging must not join the exact
        // backup payload inventory.
        let parent = crate::files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
        let name = path
            .file_name()
            .context("backup metadata has no filename")?;
        let mut file = parent.create_new(name)?;
        use std::io::Write;
        file.write_all(&encoded.0)?;
        file.sync_all()?;
        parent.verify(name, &file)?;
        parent.revalidate()?;
        Ok(())
    }

    pub(crate) fn verify_image(&self, image: &Path) -> Result<()> {
        self.validate_fields()?;
        ensure!(
            pinned_nbs_root(image, Privacy::OwnerOnly)? == self.dataset_root,
            "Dolt backup dataset root differs from the checked manifest"
        );
        ensure!(
            inventory(image)? == (self.directories.clone(), self.files.clone()),
            "Dolt backup directory or file inventory differs from the checked manifest"
        );
        Ok(())
    }

    fn validate_fields(&self) -> Result<()> {
        ensure!(self.format == FORMAT, "unsupported Kuru backup format");
        ensure!(
            !self.dolt_version.is_empty()
                && self.dolt_version.len() <= 64
                && self
                    .dolt_version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+')),
            "backup engine provenance is malformed"
        );
        ensure!(
            self.schema_version >= 1,
            "backup schema provenance is malformed"
        );
        ensure!(
            !self.source_project_path.is_empty() && self.source_project_path.len() <= 16 * 1024,
            "backup source project path exceeds its identity bound"
        );
        let source_digest = Sha256::digest(&self.source_project_path);
        let source_scope = format!(
            "project/{}",
            source_digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        ensure!(
            self.source_storage_scope == source_scope,
            "backup source scope differs from its canonical path"
        );
        for scope in [
            &self.source_storage_scope,
            &self.history_scope,
            &self.sql_origin_scope,
        ] {
            crate::store::project_directory(Path::new("."), scope)
                .context("backup contains an invalid project scope")?;
        }
        ensure!(
            uuid::Uuid::parse_str(&self.source_store_instance).is_ok(),
            "backup source store instance is invalid"
        );
        ensure!(
            uuid::Uuid::parse_str(&self.sql_origin_instance).is_ok(),
            "backup SQL origin instance is invalid"
        );
        ensure!(
            noms_hash(&self.dataset_root),
            "backup dataset root is not a pinned Noms hash"
        );
        let mut previous = None;
        for reference in &self.refs {
            ensure!(
                !reference.name.is_empty()
                    && reference.name.len() <= 4096
                    && !reference.name.chars().any(char::is_control),
                "backup ref name is malformed"
            );
            let key = (reference.kind, reference.name.as_str());
            ensure!(
                previous.is_none_or(|old| old < key),
                "backup ref inventory is not strictly sorted and unique"
            );
            previous = Some(key);
            ensure!(noms_hash(&reference.head), "backup ref head is malformed");
            match (reference.kind, &reference.working) {
                (BackupRefKind::Branch, Some(working)) => {
                    ensure!(
                        noms_hash(&working.working_root) && noms_hash(&working.staged_root),
                        "backup working roots are malformed"
                    );
                }
                (BackupRefKind::Tag | BackupRefKind::Remote, None) => {}
                _ => anyhow::bail!("backup ref has an invalid working-set observation"),
            }
        }
        ensure!(
            self.refs.iter().any(
                |reference| reference.kind == BackupRefKind::Branch && reference.name == "main"
            ),
            "backup omits its main ref observation"
        );
        ensure!(
            !self.files.is_empty() && self.files.len() <= MAX_FILES,
            "backup file count exceeds its metadata bound"
        );
        ensure!(
            self.directories.len() + self.files.len() <= MAX_FILES,
            "backup entry count exceeds its metadata bound"
        );
        let mut previous = None;
        for directory in &self.directories {
            validate_relative_name(directory)?;
            ensure!(
                previous.is_none_or(|name: &str| name < directory.as_str()),
                "backup directory inventory is not strictly sorted and unique"
            );
            previous = Some(directory.as_str());
            if let Some((parent, _)) = directory.rsplit_once('/') {
                ensure!(
                    self.directories.binary_search(&parent.to_owned()).is_ok(),
                    "backup directory inventory omits a parent"
                );
            }
        }
        let mut previous = None;
        for file in &self.files {
            validate_relative_name(&file.name)?;
            ensure!(
                previous.is_none_or(|name: &str| name < file.name.as_str()),
                "backup file inventory is not strictly sorted and unique"
            );
            previous = Some(&file.name);
            if let Some((parent, _)) = file.name.rsplit_once('/') {
                ensure!(
                    self.directories.binary_search(&parent.to_owned()).is_ok(),
                    "backup file inventory omits its parent directory"
                );
            }
            ensure!(
                file.sha256.len() == 64
                    && file
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "backup file digest is malformed"
            );
        }
        ensure!(
            self.files.iter().any(|file| file.name == "manifest"),
            "backup omits the Dolt dataset manifest"
        );
        Ok(())
    }
}
pub(crate) fn noms_hash(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
}

struct MetadataBuffer(Vec<u8>);

impl std::io::Write for MetadataBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_METADATA_BYTES as usize - self.0.len() {
            return Err(std::io::Error::other(
                "Kuru backup metadata exceeds its bounded format",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Drain all progress without exposing its contents. The retained prefix and
/// total byte count are diagnostics only, never proof of native completion.
#[derive(Debug)]
pub(crate) struct NativeOutput {
    pub bytes: u64,
    pub retained: Vec<u8>,
}

pub(crate) async fn drain_native_output(
    mut reader: impl AsyncRead + Unpin,
) -> Result<NativeOutput> {
    let mut output = NativeOutput {
        bytes: 0,
        retained: Vec::new(),
    };
    let mut buffer = [0_u8; BUFFER_BYTES];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        output.bytes = output
            .bytes
            .checked_add(count as u64)
            .context("native output byte count overflowed")?;
        let retain = count.min(BUFFER_BYTES - output.retained.len());
        output.retained.extend_from_slice(&buffer[..retain]);
    }
    Ok(output)
}

/// The pinned Dolt 2.3.5 NBS v5 prefix is
/// `5:__DOLT__:<lock>:<dataset-root>:`. Do not interpret table records here.
fn pinned_nbs_root(image: &Path, privacy: Privacy) -> Result<String> {
    // Native Dolt may create mode-0755 inner directories beneath Kuru's
    // owner-private stage. Inherited checks retain handle/symlink identity;
    // the enclosing stage remains the actual privacy boundary.
    let checked = crate::files::open_directory(image, privacy, NameRetention::Movable)?;
    let directory =
        crate::files::open_directory(image, Privacy::Inherited, NameRetention::Movable)?;
    let mut file = directory.read(OsStr::new("manifest"))?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while header.len() < NBS_HEADER_BYTES as usize {
        file.read_exact(&mut byte)
            .context("Dolt NBS manifest ended before its dataset root")?;
        header.push(byte[0]);
        if header.iter().filter(|&&byte| byte == b':').count() == 4 {
            break;
        }
    }
    ensure!(
        header.last() == Some(&b':'),
        "Dolt NBS manifest header exceeds the pinned bound"
    );
    directory.verify(OsStr::new("manifest"), &file)?;
    checked.revalidate()?;
    let header = std::str::from_utf8(&header).context("Dolt NBS header is not UTF-8")?;
    let fields: Vec<_> = header.split(':').collect();
    ensure!(
        fields.len() == 5 && fields[0] == "5" && fields[1] == "__DOLT__",
        "unsupported Dolt NBS manifest version"
    );
    ensure!(
        noms_hash(fields[2]) && noms_hash(fields[3]),
        "Dolt NBS manifest has a malformed lock or dataset root"
    );
    Ok(fields[3].to_owned())
}

fn validate_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= MAX_NAME_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-'
            })
            && value != "."
            && value != "..",
        "backup contains a malformed image filename"
    );
    Ok(())
}

fn validate_relative_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_RELATIVE_BYTES,
        "backup image path exceeds its bound"
    );
    let parts: Vec<_> = value.split('/').collect();
    ensure!(
        parts.len() <= MAX_DIRECTORY_DEPTH + 1,
        "backup image path exceeds its depth bound"
    );
    for part in parts {
        validate_name(part)?;
    }
    Ok(())
}

fn inventory(image: &Path) -> Result<(Vec<String>, Vec<BackupFile>)> {
    let checked = Directory::open(image, Privacy::OwnerOnly, NameRetention::Movable)?;
    let directory = Directory::open(image, Privacy::Inherited, NameRetention::Movable)?;
    let mut directories = Vec::new();
    let mut entries = BTreeMap::new();
    inventory_directory(&directory, "", &mut directories, &mut entries)?;
    directory.revalidate()?;
    checked.revalidate()?;
    directories.sort_unstable();
    Ok((directories, entries.into_values().collect()))
}

fn inventory_directory(
    directory: &Directory,
    prefix: &str,
    directories: &mut Vec<String>,
    entries: &mut BTreeMap<String, BackupFile>,
) -> Result<()> {
    for entry in fs::read_dir(directory.path())? {
        let entry = entry?;
        let filename = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("backup image filename is not UTF-8"))?;
        validate_name(&filename)?;
        let relative = if prefix.is_empty() {
            filename.clone()
        } else {
            format!("{prefix}/{filename}")
        };
        validate_relative_name(&relative)?;
        ensure!(
            directories.len() + entries.len() < MAX_FILES,
            "backup entry count exceeds its metadata bound"
        );
        if entry.file_type()?.is_dir() {
            ensure!(
                relative.split('/').count() <= MAX_DIRECTORY_DEPTH,
                "backup directory exceeds its depth bound"
            );
            let child = Directory::open(&entry.path(), Privacy::Inherited, NameRetention::Movable)?;
            ensure!(
                child.is_within(directory)?,
                "backup child directory left its checked parent"
            );
            directories.push(relative.clone());
            inventory_directory(&child, &relative, directories, entries)?;
            child.revalidate()?;
            continue;
        }
        let mut file = directory
            .read(OsStr::new(&filename))
            .with_context(|| format!("read backup image entry {relative}"))?;
        let mut hash = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; BUFFER_BYTES];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            bytes = bytes
                .checked_add(count as u64)
                .context("backup file size overflowed")?;
        }
        directory.verify(OsStr::new(&filename), &file)?;
        let digest: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        ensure!(
            entries
                .insert(
                    relative.clone(),
                    BackupFile {
                        name: relative,
                        bytes,
                        sha256: digest,
                    },
                )
                .is_none(),
            "backup image has a duplicate physical filename"
        );
    }
    directory.revalidate()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as kuru_memory;

    #[test]
    fn backup_async_fixtures_use_the_closing_scope() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        crate::test_support::assert_async_tests_run_in_closing(&root, &[root.join("backup.rs")]);
    }

    #[tokio::test]
    async fn dropped_native_restore_retains_maintenance_until_owned_cleanup() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained_stage = None;
            let mut lifecycle_root = None;
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(150), async {
                let project = root.path().join("project");
                crate::files::private_dir(&project)?;
                let project = project.canonicalize()?;
                let scope = crate::service::canonical_project_scope(&project)?;
                let options = crate::test_support::warmed_open_options(root.path().join("state"), scope.clone()).await?;
                let target = crate::store::project_directory(&options.data_dir, &scope)?;
                let maintenance = crate::service::acquire_maintenance_permit(&options).await?;
                let runtime = image_runtime(&options).await?;
                lifecycle_root = runtime.lifecycle_root.clone();
                let directory = crate::files::directory(root.path())?.create_private_directory(OsStr::new("interrupted-native-stage"))?;
                retained_stage = Some(directory.path().to_owned());
                let stage = native::ImageStage::new(directory, runtime.binary, runtime.lifecycle_root, Some(maintenance))?;
                let (launched, _release) = stage.pause_next_launch()?;
                // The image deliberately has no dataset. This fixture proves
                // cancellation ownership after real native launch, not image
                // validity; valid restore/fsck are covered separately.
                let image = root.path().join("empty-image");
                crate::files::private_dir(&image)?;
                let cancellation = BackupCancellation::default();
                let requested = cancellation.clone();
                let mut operation = Box::pin(async move { stage.restore(image, &requested).await });
                tokio::select! {
                    result = &mut operation => { result?; anyhow::bail!("native restore escaped the held launch boundary"); }
                    arrived = launched => { arrived.context("native restore did not retain its launched owner")?; }
                }
                let blocked = crate::session_driver::NativeMaintenanceLease::acquire(&options.data_dir, &scope).err().context("maintenance released while native owner was retained")?;
                ensure!(blocked.downcast_ref::<crate::session_driver::NativeMaintenanceBusy>().is_some(), "maintenance failed for another reason: {blocked:#}");
                // Drop synchronously cancels and joins the independent worker;
                // that worker reaps the same child and drains both pipes first.
                drop(operation);
                ensure!(cancellation.is_cancelled(), "dropped native caller did not request cancellation");
                let released = crate::session_driver::NativeMaintenanceLease::acquire(&options.data_dir, &scope)?;
                ensure!(!target.exists(), "dropped native restore activated the target");
                ensure!(retained_stage.as_ref().context("retained stage missing")?.exists(), "interrupted native stage was removed");
                drop(released);
                Ok::<_, anyhow::Error>(())
            }).await.context("dropped native restore fixture deadline exceeded").and_then(std::convert::identity);
            let cleanup = async {
                if let Some(stage) = retained_stage {
                    crate::test_support::await_store_quiescence(&stage, lifecycle_root.as_deref()).await?;
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!("native drop fixture cleanup also failed: {cleanup:#}"))),
            })
        }).await
    }

    #[tokio::test]
    async fn recomputed_inventory_does_not_authorize_a_corrupt_native_image() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained = None;
            let outcome = async {
                let project = root.path().join("project");
                crate::files::private_dir(&project)?;
                let project = project.canonicalize()?;
                let options = crate::test_support::warmed_open_options(
                    root.path().join("state"),
                    crate::service::canonical_project_scope(&project)?,
                )
                .await?;
                let store = crate::store::MemoryStore::open(options.clone()).await?;
                crate::test_support::closing::register(store.server_for_teardown());
                retained = Some(store.clone());
                store
                    .put(
                        "native-corruption-fixture",
                        &serde_json::json!({"retained":"unchanged source"}),
                    )
                    .await?;
                let original_head = store.revision().await?;
                let image = root.path().join("corrupt-backup");
                let cancellation = BackupCancellation::default();
                prepare(&store, &project, &image, &cancellation)
                    .await?
                    .publish(&cancellation)?;
                let mut manifest = CheckedBackup::read(&image)?.manifest;
                let chunk = manifest
                    .files
                    .iter_mut()
                    .filter(|file| file.name != "manifest" && file.bytes > 4)
                    .max_by_key(|file| file.bytes)
                    .context("native cut has no retained chunk")?;
                let chunk_path = image.join("image").join(&chunk.name);
                let mut bytes = fs::read(&chunk_path)?;
                let offset = bytes.len() / 2;
                bytes[offset] ^= 0xff;
                fs::write(&chunk_path, &bytes)?;
                chunk.sha256 = Sha256::digest(&bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                // Deliberately forge a self-consistent physical inventory in
                // this isolated fixture. Native parsing/graph validation still
                // has to reject it, without repairing or activating anything.
                fs::write(image.join("backup.json"), serde_json::to_vec(&manifest)?)?;
                let checked = CheckedBackup::read(&image)?;
                let validation = verify(&options, &image, &cancellation).await;
                ensure!(
                    validation.is_err(),
                    "recomputed physical hashes authorized a corrupt native image"
                );
                checked.verify_unchanged()?;
                ensure!(
                    store.revision().await? == original_head
                        && store.get("native-corruption-fixture").await?
                            == Some(serde_json::json!({"retained":"unchanged source"})),
                    "native verification mutated its source"
                );
                let lifecycle = store.image_runtime().lifecycle_root;
                for entry in fs::read_dir(options.data_dir.join("backup-validation"))? {
                    let path = entry?.path();
                    crate::test_support::await_store_quiescence(&path, lifecycle.as_deref())
                        .await?;
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let cleanup = async {
                if let Some(store) = retained {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref())
                        .await?;
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!(
                    "corrupt-image source cleanup also failed: {cleanup:#}"
                ))),
            })
        })
        .await
    }

    #[tokio::test]
    async fn backup_client_eof_and_lost_published_reply_have_distinct_checked_outcomes()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            use crate::{MemoryStore, service, test_support};
            use std::time::Duration;

            test_support::warm_runtime_cache().await?;
            let root = test_support::tempdir()?;
            let project = root.path().join("project");
            crate::files::private_dir(&project)?;
            let project = project.canonicalize()?;
            let options = test_support::warmed_open_options(root.path().join("state"), service::canonical_project_scope(&project)?).await?;
            let deadline = test_support::FixtureDeadline::start(Duration::from_secs(150), "backup client EOF fixture");
            let outcome = deadline.serve(async |served| {
                let _gate = crate::spawn_gate::spawning().await;
                let owner = service::ServiceOwner::open(options.clone(), &project).await?;
                let inspection = owner.inspection_store_for_test();
                let (knobs, mut events) = test_support::observed(service::Admission::Never, None);
                served.serve_with(owner, knobs)?;
                let executable = std::env::current_exe()?;
                let peer = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable.clone()).1.await?;
                peer.put("before-backup", &serde_json::json!(true)).await?;
                for published in [false, true] {
                    let target = root.path().join(if published { "confirmed-destination" } else { "cancelled-destination" });
                    let mut attachment = service::attach_or_start(&options, &project, &executable).await?;
                    let mut held_cut_release = None;
                    if published {
                        let pause = Arc::new(service::rpc::ReplyPause::default());
                        let replied = pause.replied.notified();
                        attachment.pause_after_next_send(pause.clone());
                        let operation = attachment.call(service::ServiceCall::Backup { target: target.clone() });
                        tokio::pin!(operation);
                        tokio::select! {
                            result = &mut operation => {
                                result?;
                                anyhow::bail!("backup reply escaped its held delivery boundary");
                            }
                            () = replied => {},
                        }
                        // The real handler published and wrote its reply. Drop
                        // that client result rather than resending the operation.
                    } else {
                        let (completed, release) = inspection.server_for_teardown().pause_next_backup_completed_cut()?;
                        held_cut_release = Some(release);
                        let operation = attachment.call(service::ServiceCall::Backup { target: target.clone() });
                        tokio::pin!(operation);
                        tokio::select! {
                            result = &mut operation => {
                                result?;
                                anyhow::bail!("backup escaped its pre-validation boundary");
                            }
                            result = completed => result.context("backup did not finish its SQL cut")?,
                        }
                    }
                    attachment.close();
                    // This joined event follows the exact connection handler's
                    // awaited EOF cleanup, not an elapsed grace or socket count.
                    loop {
                        if matches!(test_support::next_event(&mut events).await?, service::ServeEvent::AttachmentJoined { remaining: 1 }) {
                            break;
                        }
                    }
                    drop(held_cut_release);
                    peer.put("after-disconnected-backup", &serde_json::json!(published)).await?;
                    ensure!(peer.get("after-disconnected-backup").await? == Some(serde_json::json!(published)), "backup EOF closed its authenticated peer");
                    ensure!(target.exists() == published, "EOF changed publication authority at the wrong boundary");
                    if published {
                        let verified = MemoryStore::verify_backup(options.clone(), &target, &BackupCancellation::default()).await?;
                        ensure!(verified == BackupResult::from_manifest(&CheckedBackup::read(&target)?.manifest)?, "independent verification could not recover the published destination");
                    } else {
                        let stages: Vec<_> = fs::read_dir(root.path())?.map(|entry| entry.map(|entry| entry.path())).collect::<std::io::Result<Vec<_>>>()?.into_iter().filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("kuru-backup-stage-"))).collect();
                        ensure!(stages.len() == 1 && stages[0].join("image").is_dir() && !stages[0].join("backup.json").exists(), "cancelled client image was deleted, hashed or published");
                    }
                }
                peer.close().await?;
                drop(inspection);
                Ok::<_, anyhow::Error>(())
            }, async |served| {
                served.retire(&options, None, Duration::from_secs(15), "backup EOF fixture owner did not reap").await
            }).await;
            root.release(outcome)
        }).await
    }

    #[tokio::test]
    async fn unconfirmed_cut_and_cancelled_validation_retain_unpublished_private_images()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained = None;
            let outcome = async {
                let project = root.path().join("project");
                crate::files::private_dir(&project)?;
                let project = project.canonicalize()?;
                let scope = crate::service::canonical_project_scope(&project)?;
                let options = crate::test_support::warmed_open_options(root.path().join("state"), scope).await?;
                let store = crate::store::MemoryStore::open(options).await?;
                crate::test_support::closing::register(store.server_for_teardown());
                retained = Some(store.clone());
                store.put("cut-sentinel", &serde_json::json!(true)).await?;
                for discard_reply in [true, false] {
                    let target = root.path().join(if discard_reply { "unconfirmed" } else { "cancelled" });
                    let cancellation = BackupCancellation::default();
                    let result = if discard_reply {
                        store.server_for_teardown().discard_next_backup_reply();
                        prepare(&store, &project, &target, &cancellation).await
                    } else {
                        let (completed, _release) = store.server_for_teardown().pause_next_backup_completed_cut()?;
                        let operation = prepare(&store, &project, &target, &cancellation);
                        tokio::pin!(operation);
                        tokio::select! {
                            result = &mut operation => {
                                result?;
                                anyhow::bail!("backup prepared before its held completion boundary");
                            }
                            result = completed => result.context("cut did not reach positive SQL completion")?,
                        }
                        cancellation.cancel();
                        operation.await
                    };
                    let error = result.err().context("unconfirmed/cancelled backup was prepared")?;
                    ensure!(format!("{error:#}").contains(if discard_reply { "discarded native backup CALL reply" } else { "cancelled" }), "wrong cut outcome: {error:#}");
                    ensure!(!target.exists(), "unconfirmed/cancelled cut was published");
                    let stages: Vec<_> = fs::read_dir(root.path())?.map(|entry| entry.map(|entry| entry.path())).collect::<std::io::Result<Vec<_>>>()?.into_iter().filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("kuru-backup-stage-"))).collect();
                    ensure!(stages.len() == if discard_reply { 1 } else { 2 }, "unfinished private cut was removed or replaced");
                    for stage in stages {
                        let directory = Directory::open(&stage, Privacy::OwnerOnly, NameRetention::Pinned)?;
                        ensure!(stage.join("image").is_dir() && !stage.join("backup.json").exists(), "unconfirmed image was deleted or acquired publication metadata");
                        directory.revalidate()?;
                    }
                    store.put("surviving-peer", &serde_json::json!(discard_reply)).await?;
                    ensure!(store.get("surviving-peer").await? == Some(serde_json::json!(discard_reply)), "failed backup disabled ordinary source writes");
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            let cleanup = async {
                if let Some(store) = retained {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref()).await?;
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!("unconfirmed-cut source cleanup also failed: {cleanup:#}"))),
            })
        }).await
    }

    #[tokio::test]
    async fn managed_backup_keeps_writer_admitted_and_restores_the_captured_cut() -> Result<()> {
        kuru_memory::test_support::closing(async {
            use crate::{MemoryStore, SessionDriverTarget, service, test_support};
            use kuru_core::{Message, Mode};
            use std::time::Duration;

            test_support::warm_runtime_cache().await?;
            let root = test_support::tempdir()?;
            let project = root.path().join("project");
            let restored_project = root.path().join("offline-project");
            crate::files::private_dir(&project)?;
            crate::files::private_dir(&restored_project)?;
            let project = project.canonicalize()?;
            let restored_project = restored_project.canonicalize()?;
            let scope = service::canonical_project_scope(&project)?;
            let options = test_support::warmed_open_options(root.path().join("state"), scope.clone()).await?;
            let deadline = test_support::FixtureDeadline::start(Duration::from_secs(150), "managed concurrent backup fixture");
            let mut restored = None;
            let outcome = deadline.serve(async |served| {
                let _gate = crate::spawn_gate::spawning().await;
                let owner = service::ServiceOwner::open(options.clone(), &project).await?;
                let inspection = owner.inspection_store_for_test();
                let (cut_finished, release_cut) = inspection.server_for_teardown().pause_next_backup_completed_cut()?;
                served.serve(owner)?;
                let executable = std::env::current_exe()?;
                let memory = MemoryStore::open_managed_observed(options.clone(), project.clone(), executable).1.await?;
                memory.create_session("writer", Mode::Ifs, "backup writer").await?;
                let (writer, driver) = memory.bind_session_driver(&options.data_dir, &project).await?;
                driver.select(SessionDriverTarget::Catalog(Box::new(memory.session_catalog_record("writer").await?.context("writer catalog missing")?))).await?;
                memory.create_session("sibling", Mode::Ifs, "independent backup writer").await?;
                let (sibling, sibling_driver) = memory.bind_session_driver(&options.data_dir, &project).await?;
                sibling_driver.select(SessionDriverTarget::Catalog(Box::new(memory.session_catalog_record("sibling").await?.context("sibling catalog missing")?))).await?;
                let actor = format!("{scope}/ifs/identity/actor");
                let target = root.path().join("backup");
                let cancellation = BackupCancellation::default();
                let (writer_ready, start_backup) = tokio::sync::oneshot::channel();
                let (published, after_publication) = tokio::sync::oneshot::channel();

                // Two independent drivers remain selected across capture and publication.
                // The native completion pause follows SQL's response AND Sleep;
                // its write ACK proves Rust ownership is free, not in-copy overlap.
                let writing = async {
                    writer.append_session_message(&actor, "writer", &Message::text("user", "captured private history")).await?;
                    sibling.append_session_message(&actor, "sibling", &Message::text("user", "captured sibling private history")).await?;
                    writer.usage_ledger()?.mark_new_session("captured-usage").await?;
                    let before = writer.revision().await?;
                    let usage = inspection.server_for_teardown().pool("kuru_usage_v1").await?;
                    let usage_before: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(usage.as_ref()).await?;
                    writer_ready.send((before.clone(), usage_before)).map_err(|_| anyhow::anyhow!("backup stopped before writer readiness"))?;
                    cut_finished.await.context("backup did not reach positively completed native cut")?;
                    ensure!(writer.live_session_drivers().await?.len() == 2, "backup displaced an ordinary session driver");
                    let after_cut_message = Message::text("user", "after captured cut");
                    let sibling_after_cut = Message::text("user", "sibling after captured cut");
                    tokio::try_join!(
                        writer.append_session_message(&actor, "writer", &after_cut_message),
                        sibling.append_session_message(&actor, "sibling", &sibling_after_cut),
                    )?;
                    writer.usage_ledger()?.mark_new_session("after-cut-usage").await?;
                    let after_cut = writer.revision().await?;
                    ensure!(after_cut != before, "writer ACK did not commit while completed cut was held");
                    release_cut.send(()).map_err(|_| anyhow::anyhow!("backup left its completed cut before writer ACK"))?;
                    after_publication.await.context("backup did not publish")?;
                    writer.append_session_message(&actor, "writer", &Message::text("user", "after backup publication")).await?;
                    sibling.append_session_message(&actor, "sibling", &Message::text("user", "sibling after backup publication")).await?;
                    ensure!(writer.session_history_window(&actor, "writer", 8).await?.total_rows == 3, "backup interrupted finite writer history");
                    ensure!(sibling.session_history_window(&actor, "sibling", 8).await?.total_rows == 3, "backup interrupted independent sibling history");
                    Ok::<_, anyhow::Error>(())
                };
                let capturing = async {
                    let (head, usage_head) = start_backup.await.context("writer did not become ready")?;
                    let result = memory.backup(&project, &target, &cancellation).await?;
                    published.send(()).map_err(|_| anyhow::anyhow!("writer left before backup publication"))?;
                    Ok::<_, anyhow::Error>((result, head, usage_head))
                };
                let ((), (result, head, usage_head)) = tokio::try_join!(writing, capturing)?;
                let checked = CheckedBackup::read(&target)?;
                for (branch, expected) in [("main", head), ("kuru_usage_v1", usage_head)] {
                    ensure!(checked.manifest.refs.iter().any(|reference| reference.kind == BackupRefKind::Branch && reference.name == branch && reference.head == expected), "native cut changed the captured {branch} head");
                }
                ensure!(MemoryStore::verify_backup(options.clone(), &target, &cancellation).await? == result, "independent verification changed backup completion metadata");
                driver.close().await?;
                sibling_driver.close().await?;
                writer.close().await?;
                sibling.close().await?;
                memory.close().await?;
                drop(inspection);
                served.retire(&options, None, Duration::from_secs(15), "backup source owner did not retire").await?;

                // Restore after the source is reaped: no source connection can
                // fill gaps in the captured history or same-dataset usage ref.
                let mut target_options = options.clone();
                target_options.project_scope = service::canonical_project_scope(&restored_project)?;
                MemoryStore::restore_backup(target_options.clone(), &restored_project, &target, true, &cancellation).await?;
                let offline = crate::store::MemoryStore::open(target_options).await?;
                test_support::closing::register(offline.server_for_teardown());
                restored = Some(offline.clone());
                let history = offline.session_history_window(&actor, "writer", 8).await?;
                ensure!(history.total_rows == 1 && history.messages[0].plain_text() == Some("captured private history"), "offline restore included later writer history or lost the captured private row");
                let sibling_history = offline.session_history_window(&actor, "sibling", 8).await?;
                ensure!(sibling_history.total_rows == 1 && sibling_history.messages[0].plain_text() == Some("captured sibling private history"), "offline restore interleaved or lost independent private history");
                let usage = offline.server_for_teardown().pool("kuru_usage_v1").await?;
                for (session, count) in [("captured-usage", 1_i64), ("after-cut-usage", 0)] {
                    let digest: String = Sha256::digest(session.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
                    let observed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM state WHERE `key` = ?").bind(format!("kuru.usage.v1/session/{digest}").as_bytes()).fetch_one(usage.as_ref()).await?;
                    ensure!(observed == count, "offline native usage cut does not match its captured head");
                }
                ensure!(offline.session_catalog_record("writer").await?.is_some(), "offline cut lost the session catalog");
                checked.verify_unchanged()?;
                Ok::<_, anyhow::Error>(())
            }, async |served| {
                served.retire(&options, None, Duration::from_secs(15), "concurrent backup fixture owner did not reap").await
            }).await;
            let cleanup = async {
                if let Some(store) = restored {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    test_support::await_store_quiescence(&directory, lifecycle.as_deref()).await?;
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!("offline restore cleanup also failed: {cleanup:#}"))),
            })
        }).await
    }

    #[tokio::test]
    async fn historical_dirty_restore_migrates_actual_working_values_without_rewriting_image()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained_server = None;
            let mut retained_store = None;
            let outcome = async {
                let project = root.path().join("historical-project");
                let target_project = root.path().join("migrated-project");
                crate::files::private_dir(&project)?;
                crate::files::private_dir(&target_project)?;
                let project = project.canonicalize()?;
                let target_project = target_project.canonicalize()?;
                let scope = crate::service::canonical_project_scope(&project)?;
                let target_scope = crate::service::canonical_project_scope(&target_project)?;
                let options = crate::test_support::warmed_open_options(
                    root.path().join("state"),
                    scope.clone(),
                )
                .await?;
                crate::store::released_v1(&options).await?;
                let directory = crate::store::project_directory(&options.data_dir, &scope)?;
                let runtime = image_runtime(&options).await?;
                let server = crate::server::Server::open(crate::server::ServerOptions {
                    expected_instance: None,
                    binary: runtime.binary.clone(),
                    directory: directory.clone(),
                    project_scope: scope.clone(),
                    supervisor: runtime.supervisor.clone(),
                    timeout: std::time::Duration::from_secs(options.config.startup_timeout_secs),
                    read_only: false,
                    retained: None,
                    lifecycle_root: runtime.lifecycle_root.clone(),
                    ticks: None,
                })
                .await?;
                crate::test_support::closing::register(server.clone());
                retained_server = Some((server.clone(), directory, runtime.lifecycle_root.clone()));
                let main = server.pool("main").await?;
                let key = format!("{scope}/historical-private-state");
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                    .bind(key.as_bytes())
                    .bind("\"committed\"")
                    .execute(main.as_ref())
                    .await?;
                sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                    .bind(format!("{scope}/history").as_bytes())
                    .bind(b"assistant".as_slice())
                    .bind("historical private text")
                    .execute(main.as_ref())
                    .await?;
                sqlx::query("CALL DOLT_COMMIT('-Am', 'Historical retained values', '--author', ?)")
                    .bind(crate::store::AUTHOR)
                    .fetch_all(main.as_ref())
                    .await?;
                let original_head: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                    .fetch_one(main.as_ref())
                    .await?;
                for (value, staged) in [("staged", true), ("actual-working", false)] {
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind(serde_json::to_string(value)?)
                        .bind(key.as_bytes())
                        .execute(main.as_ref())
                        .await?;
                    if staged {
                        sqlx::query("CALL DOLT_ADD('state')")
                            .execute(main.as_ref())
                            .await?;
                    }
                }
                let backup = root.path().join("historical-backup");
                let parent = crate::files::directory(root.path())?;
                let backup_dir =
                    parent.create_private_directory(OsStr::new("historical-backup"))?;
                let image = backup_dir.create_private_directory(OsStr::new("image"))?;
                let cancel = BackupCancellation::default();
                capture::copy(
                    server.backup_connection()?,
                    image.path().to_owned(),
                    &cancel,
                )
                .await?;
                let (origin_instance, origin_scope) = server.sql_origin();
                let source = BackupSource {
                    project_path: crate::service::project_path_bytes(&project),
                    storage_scope: scope.clone(),
                    history_scope: scope.clone(),
                    store_instance: server.instance().into(),
                    sql_origin_instance: origin_instance.into(),
                    sql_origin_scope: origin_scope.into(),
                };
                let inspection =
                    inspect_image(runtime, &parent, image.path(), source.clone(), &cancel).await?;
                ensure!(
                    inspection.schema_version == 1,
                    "released input was upgraded before original validation"
                );
                let manifest = BackupManifest::new(
                    source,
                    inspection.schema_version,
                    inspection.refs,
                    image.path(),
                )?;
                manifest.write(&backup_dir.path().join("backup.json"))?;
                let checked = CheckedBackup::read(&backup)?;
                let mut target_options = options.clone();
                target_options.project_scope = target_scope;
                let result = restore(
                    target_options.clone(),
                    &target_project,
                    &backup,
                    true,
                    &cancel,
                )
                .await?;
                ensure!(
                    result.schema_version > manifest.schema_version
                        && result.source_dataset_root == manifest.dataset_root,
                    "historical restore did not migrate from its original cut"
                );
                let restored = crate::store::MemoryStore::open(target_options).await?;
                crate::test_support::closing::register(restored.server_for_teardown());
                retained_store = Some(restored.clone());
                ensure!(
                    restored.schema_version().await? == result.schema_version
                        && restored.get(&key).await? == Some(serde_json::json!("actual-working")),
                    "migration used committed or staged values instead of actual WORKING"
                );
                let pool = restored.server_for_teardown().pool("main").await?;
                let ancestor: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?")
                        .bind(&original_head)
                        .fetch_one(pool.as_ref())
                        .await?;
                let content: (String, String) = sqlx::query_as(
                    "SELECT content, content_format FROM messages WHERE namespace = ?",
                )
                .bind(format!("{scope}/history").as_bytes())
                .fetch_one(pool.as_ref())
                .await?;
                ensure!(
                    ancestor == 1
                        && content == ("historical private text".into(), "text-v1".into()),
                    "migration lost original history or released message interpretation"
                );
                restored
                    .put(&format!("{scope}/post-migration"), &serde_json::json!(true))
                    .await?;
                checked.verify_unchanged()?;
                Ok(())
            }
            .await;
            let cleanup = async {
                if let Some(store) = retained_store {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref())
                        .await?;
                }
                if let Some((server, directory, lifecycle)) = retained_server {
                    server.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref())
                        .await?;
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!(
                    "historical restore fixture cleanup also failed: {cleanup:#}"
                ))),
            })
        })
        .await
    }

    #[tokio::test]
    async fn remapped_restore_preserves_dirty_main_usage_snapshots_and_fresh_authority()
    -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained = Vec::new();
            let outcome = async {
                let project = root.path().join("source-project");
                let target_project = root.path().join("target-project");
                crate::files::private_dir(&project)?;
                crate::files::private_dir(&target_project)?;
                let project = project.canonicalize()?;
                let target_project = target_project.canonicalize()?;
                let scope = crate::service::canonical_project_scope(&project)?;
                let target_scope = crate::service::canonical_project_scope(&target_project)?;
                let options = crate::test_support::warmed_open_options(root.path().join("state"), scope.clone()).await?;
                let source = crate::store::MemoryStore::open(options.clone()).await?;
                crate::test_support::closing::register(source.server_for_teardown());
                retained.push(source.clone());
                let key = format!("{scope}/retained-private-summary");
                source.put(&key, &serde_json::json!("committed-private")).await?;
                let actor = format!("{scope}/ifs/identity/restore-context");
                let session = "restore-private-context";
                source.append_session_message(&actor, session, &kuru_core::Message::text("user", "retained private context")).await?;
                let captured = source.session_source_snapshot(&actor, session, &actor, 0, 16).await?;
                let summary_record = crate::store::ContextSummaryRecord {
                    actor_namespace: actor.clone(),
                    session_id: session.into(),
                    source_namespace: actor.clone(),
                    summary_namespace: format!("{actor}/summaries"),
                    source_view: captured.view,
                    source_revision: captured.revision,
                    after_sequence: 0,
                    through_sequence: captured.through_inclusive.context("private summary source is empty")?,
                    turn_id: Some("restore-context-turn".into()),
                    operation_id: None,
                    producer_actor_id: None,
                    invocation_id: "restore-context-invocation".into(),
                    summary: "retained private summary body".into(),
                };
                source.checkpoint_context_summary(&crate::store::ContextSummaryCheckpoint {
                    record: summary_record.clone(),
                    private_reasoning: vec![],
                }).await?;
                source.usage_ledger()?.mark_new_session("restore-usage").await?;
                let server = source.server_for_teardown();
                let main = server.pool("main").await?;
                for (value, stage) in [("staged-private", true), ("working-private", false)] {
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind(serde_json::to_string(value)?).bind(key.as_bytes())
                        .execute(main.as_ref()).await?;
                    if stage {
                        sqlx::query("CALL DOLT_ADD('state')").execute(main.as_ref()).await?;
                    }
                }
                let usage = server.pool("kuru_usage_v1").await?;
                let usage_key = format!("kuru.usage.v1/session/{}", Sha256::digest(b"restore-usage").iter().map(|byte| format!("{byte:02x}")).collect::<String>());
                for (complete, stage) in [(false, true), (true, false)] {
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind(serde_json::to_string(&serde_json::json!({"format":1,"session_id":"restore-usage","historical_complete":complete}))?)
                        .bind(usage_key.as_bytes()).execute(usage.as_ref()).await?;
                    if stage {
                        sqlx::query("CALL DOLT_ADD('state')").execute(usage.as_ref()).await?;
                    }
                }
                // Keep a non-main native reference too; target preparation
                // must not change its captured head or working coordinates.
                let candidate = source.begin_candidate("restored retained candidate").await?;
                let candidate_name = candidate.branch().to_owned();
                let backup = root.path().join("backup");
                let cancel = BackupCancellation::default();
                prepare(&source, &project, &backup, &cancel).await?.publish(&cancel)?;
                let checked = CheckedBackup::read(&backup)?;
                let source_instance = source.service_instance().to_owned();
                let mut target_options = options.clone();
                target_options.project_scope = target_scope.clone();
                ensure!(restore(target_options.clone(), &target_project, &backup, false, &cancel).await.is_err(), "remap was accepted without explicit selection");
                let active = Arc::new(crate::session_driver::NativeSessionLease::acquire(&target_options.data_dir, &target_project, "held-restore-driver")?);
                let retained_work = active.clone();
                let mut active = Some(active);
                for draining in [false, true] {
                    let refused = restore(target_options.clone(), &target_project, &backup, true, &cancel).await.err().context("restore crossed an active or draining native driver")?;
                    ensure!(refused.downcast_ref::<crate::session_driver::NativeMaintenanceBusy>().is_some(), "restore refused for another reason: {refused:#}");
                    ensure!(!crate::store::project_directory(&target_options.data_dir, &target_scope)?.exists(), "refused restore created its target");
                    if !draining { drop(active.take()); }
                }
                drop(retained_work);
                let result = restore(target_options.clone(), &target_project, &backup, true, &cancel).await?;
                ensure!(result.source_dataset_root == checked.manifest.dataset_root && result.schema_version == checked.manifest.schema_version, "restore completion lost the selected original cut");
                checked.verify_unchanged()?;
                // Recreate the recoverable ready-stage position while keeping
                // the exact stopped directory and maintenance ownership.
                let target = crate::store::project_directory(&target_options.data_dir, &target_scope)?;
                let target_name = target.file_name().and_then(OsStr::to_str).context("restore target name is not UTF-8")?;
                let ready_stage = target.with_file_name(format!("{target_name}.staging-{}", uuid::Uuid::new_v4()));
                let maintenance = crate::service::acquire_maintenance_permit(&target_options).await?;
                let lifecycle = source.image_runtime().lifecycle_root;
                let mut seal = crate::server::Server::quiescence_at(&target, lifecycle.as_deref(), crate::server::close_budget()).await?;
                seal.move_to(&ready_stage)?;
                drop(seal);
                drop(maintenance);
                ensure!(!target.exists() && ready_stage.join("ready.json").exists(), "ready restore stage was not retained before recovery");
                let restored = crate::store::MemoryStore::open(target_options).await?;
                crate::test_support::closing::register(restored.server_for_teardown());
                retained.push(restored.clone());
                ensure!(target.exists() && !ready_stage.exists(), "ordinary reopen did not recover the ready restore stage");
                ensure!(restored.service_instance() != source_instance && restored.history_scope() == scope, "restore reused source authority or lost its history namespace");
                ensure!(restored.get(&key).await? == Some(serde_json::json!("working-private")), "restore did not retain actual working private values");
                let summaries = restored.context_summary_window(&actor, &summary_record.summary_namespace, Some(session), Some(&actor), 16).await?;
                ensure!(summaries.total_rows == 1 && summaries.records.len() == 1 && summaries.records[0].record == summary_record, "ready-stage recovery changed private summary body or provenance");
                let directory = restored.status().await?.directory;
                let activation: serde_json::Value = serde_json::from_slice(&fs::read(directory.join("ready.json"))?)?;
                let snapshots = activation["restore"]["prepared_roots"].as_array().context("restore snapshot receipt missing")?;
                ensure!(snapshots.len() == 2, "dirty main/usage snapshots were not both recorded");
                let restored_server = restored.server_for_teardown();
                for snapshot in snapshots {
                    let branch = snapshot["branch"].as_str().context("snapshot branch missing")?;
                    let original = checked.manifest.refs.iter().find(|reference| reference.kind == BackupRefKind::Branch && reference.name == branch).context("original snapshot ref missing")?;
                    let roots = original.working.as_ref().context("original snapshot working roots missing")?;
                    ensure!(snapshot["original_head"] == original.head && snapshot["original_staged"] == roots.staged_root && snapshot["original_working"] == roots.working_root && snapshot["staged_commit"].as_str().is_some(), "snapshot provenance lost the distinct original roots");
                    let pool = restored_server.pool(branch).await?;
                    let ancestors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?").bind(&original.head).fetch_one(pool.as_ref()).await?;
                    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(pool.as_ref()).await?;
                    ensure!(ancestors == 1 && dirty == 0, "prepared writable branch lost original ancestry or remains dirty");
                }
                let original_candidate = checked.manifest.refs.iter().find(|reference| reference.name == candidate_name).context("original candidate ref missing")?;
                let candidate_pool = restored_server.pool(&candidate_name).await?;
                let candidate_head: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(candidate_pool.as_ref()).await?;
                ensure!(candidate_head == original_candidate.head, "restore preparation changed a candidate ref");
                restored.put(&format!("{scope}/after-restore"), &serde_json::json!("accepted")).await?;
                restored.usage_ledger()?.mark_new_session("after-restore").await?;
                checked.verify_unchanged()?;
                Ok(())
            }.await;
            let cleanup = async {
                for store in retained {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref()).await?;
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => Err(error.context(format!("restore fixture cleanup also failed: {cleanup:#}"))),
            })
        }).await
    }

    #[tokio::test]
    async fn prepared_backup_publishes_once_and_verifies_without_changing_the_image() -> Result<()>
    {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let mut retained = None;
            let outcome = async {
                let project = root.path().join("project");
                crate::files::private_dir(&project)?;
                let project = project.canonicalize()?;
                let scope = crate::service::canonical_project_scope(&project)?;
                let options =
                    crate::test_support::warmed_open_options(root.path().join("state"), scope)
                        .await?;
                let store = crate::store::MemoryStore::open(options.clone()).await?;
                crate::test_support::closing::register(store.server_for_teardown());
                retained = Some(store.clone());
                store
                    .put(
                        "prepared-backup-report",
                        &serde_json::json!({"value":"cut"}),
                    )
                    .await?;
                let head = store.revision().await?;
                let cancellation = BackupCancellation::default();
                let target = root.path().join("backup");
                let prepared = prepare(&store, &project, &target, &cancellation).await?;
                ensure!(!target.exists(), "prepare activated the backup prematurely");
                let staged = CheckedBackup::read(prepared.retained_path())?;
                ensure!(
                    staged
                        .manifest
                        .refs
                        .iter()
                        .any(|reference| reference.name == "main" && reference.head == head),
                    "prepared cut lost its captured main head"
                );
                let result = prepared.publish(&cancellation)?;
                let checked = CheckedBackup::read(&target)?;
                ensure!(
                    checked.manifest == staged.manifest,
                    "publication changed validated metadata"
                );
                store
                    .put(
                        "prepared-backup-report",
                        &serde_json::json!({"value":"later"}),
                    )
                    .await?;
                let later = store.revision().await?;
                ensure!(
                    later != head,
                    "live writer did not advance after publication"
                );
                let verified = verify(&options, &target, &cancellation).await?;
                ensure!(
                    verified == result,
                    "independent verification changed completion metadata"
                );
                checked.verify_unchanged()?;
                ensure!(
                    store.revision().await? == later,
                    "verification changed the live source"
                );
                // Successful private validation stages are removed, rather
                // than being included in the payload inventory or activation.
                for entry in fs::read_dir(root.path())? {
                    ensure!(
                        !entry?
                            .file_name()
                            .to_string_lossy()
                            .starts_with("kuru-backup-validation-"),
                        "settled validation stage was retained after success"
                    );
                }
                ensure!(
                    prepare(&store, &project, &target, &cancellation)
                        .await
                        .is_err(),
                    "existing backup destination was replaced"
                );
                checked.verify_unchanged()?;
                let before_cancel = root.path().join("cancelled");
                cancellation.cancel();
                ensure!(
                    prepare(&store, &project, &before_cancel, &cancellation)
                        .await
                        .is_err()
                        && !before_cancel.exists(),
                    "cancelled preparation activated a destination"
                );
                Ok(())
            }
            .await;
            let cleanup = async {
                if let Some(store) = retained.take() {
                    let directory = store.status().await?.directory;
                    let lifecycle = store.image_runtime().lifecycle_root;
                    store.close().await?;
                    crate::test_support::await_store_quiescence(&directory, lifecycle.as_deref())
                        .await?;
                }
                // A rejected operation deliberately retains its private
                // image/stage. Record actual settled owners before releasing
                // this fixture root, without removing diagnostic evidence.
                for parent in [
                    root.path().to_owned(),
                    root.path().join("state/backup-validation"),
                ] {
                    if !parent.exists() {
                        continue;
                    }
                    for entry in fs::read_dir(parent)? {
                        let entry = entry?;
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("kuru-backup-validation-")
                        {
                            #[cfg(unix)]
                            let lifecycle: Option<std::path::PathBuf> = None;
                            #[cfg(windows)]
                            let lifecycle = Some(root.path().join("state/memory/lifecycles"));
                            crate::test_support::await_store_quiescence(
                                &entry.path(),
                                lifecycle.as_deref(),
                            )
                            .await?;
                        }
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            root.release(match (outcome, cleanup) {
                (outcome, Ok(())) => outcome,
                (Ok(()), Err(cleanup)) => Err(cleanup),
                (Err(error), Err(cleanup)) => {
                    Err(error.context(format!("backup fixture cleanup also failed: {cleanup:#}")))
                }
            })
        })
        .await
    }

    #[tokio::test]
    async fn pinned_native_copy_restore_fsck_preserve_the_completed_dataset_root() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let stage_path = root.path().join("validation");
            #[cfg(unix)]
            let lifecycle_root = None;
            #[cfg(windows)]
            let lifecycle_root = Some(root.path().join("lifecycles"));
            let outcome = async {
                let store = crate::store::MemoryStore::temporary_cold().await?;
                crate::test_support::closing::register(store.server_for_teardown());
                store
                    .put(
                        "native-backup-fixture",
                        &serde_json::json!({"revision":"captured"}),
                    )
                    .await?;
                let pool = store.server_for_teardown().pool("main").await?;
                for selected in ["HEAD", "WORKING", "STAGED"] {
                    let hash: String = crate::pool::within(
                        crate::store::QUERY_TIMEOUT,
                        sqlx::query_scalar("SELECT DOLT_HASHOF_DB(?)")
                            .bind(selected)
                            .fetch_one(pool.as_ref()),
                    )
                    .await
                    .context("pinned database-root read exceeded its query budget")??;
                    ensure!(
                        noms_hash(&hash),
                        "pinned database root selection {selected}"
                    );
                }
                let image = root.path().join("image");
                crate::files::private_dir(&image)?;
                let cancellation = BackupCancellation::default();
                capture::copy(store.backup_connection()?, image.clone(), &cancellation).await?;
                let cut = pinned_nbs_root(&image, Privacy::OwnerOnly)?;
                store
                    .put(
                        "native-backup-fixture",
                        &serde_json::json!({"revision":"later"}),
                    )
                    .await?;
                let binary = crate::provision::provision(
                    &kuru_core::MemoryConfig::default(),
                    &root.path().join("engine-cache"),
                )
                .await?;
                let parent = crate::files::directory(root.path())?;
                let stage = parent.create_private_directory(OsStr::new("validation"))?;
                let stage = native::ImageStage::new(stage, binary, lifecycle_root.clone(), None)?;
                stage
                    .restore(image.clone(), &cancellation)
                    .await
                    .context("pinned native restore failed")?;
                ensure!(
                    !cancellation.is_cancelled(),
                    "successful restore leaves fsck enabled"
                );
                ensure!(
                    pinned_nbs_root(
                        &stage.directory.path().join("data/kuru/.dolt/noms"),
                        Privacy::Inherited
                    )? == cut,
                    "native restored dataset root differs from the completed cut"
                );
                stage
                    .fsck(&cancellation)
                    .await
                    .context("pinned native fsck failed")?;
                ensure!(
                    pinned_nbs_root(&image, Privacy::OwnerOnly)? == cut,
                    "native validation changed the source image root"
                );
                drop(pool);
                store.close().await?;
                Ok(())
            }
            .await;
            // The native worker returns only after its retained process and
            // output readers settle. Record the lease-only stage through the
            // same awaited lifecycle check used by other owned fixtures.
            let cleanup = if stage_path.exists() {
                crate::test_support::await_store_quiescence(&stage_path, lifecycle_root.as_deref())
                    .await
            } else {
                Ok(())
            };
            let outcome = match (outcome, cleanup) {
                (Ok(()), cleanup) => cleanup,
                (Err(error), Ok(())) => Err(error),
                (Err(error), Err(cleanup)) => {
                    Err(error.context(format!("native fixture cleanup also failed: {cleanup:#}")))
                }
            };
            root.release(outcome)
        })
        .await
    }

    #[tokio::test]
    async fn native_sql_validation_preserves_dirty_candidate_and_usage_refs() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let stage_path = root.path().join("validation");
            #[cfg(unix)]
            let lifecycle_root = None;
            #[cfg(windows)]
            let lifecycle_root = Some(root.path().join("lifecycles"));
            let outcome = async {
                let store = crate::store::MemoryStore::temporary_cold().await?;
                let server = store.server_for_teardown();
                crate::test_support::closing::register(server.clone());
                store
                    .put("dirty-image-marker", &serde_json::json!("committed"))
                    .await?;
                store
                    .usage_ledger()?
                    .mark_new_session("backup-usage")
                    .await?;
                let candidate = store.begin_candidate("backup-validation").await?;
                let candidate_name = candidate.branch().to_owned();
                let candidate_pool = server.pool(&candidate_name).await?;
                crate::pool::within(crate::store::QUERY_TIMEOUT, async {
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind(serde_json::to_string(&serde_json::json!("staged"))?)
                        .bind(b"dirty-image-marker".as_slice())
                        .execute(candidate_pool.as_ref())
                        .await?;
                    sqlx::query("CALL DOLT_ADD(?)")
                        .bind("state")
                        .execute(candidate_pool.as_ref())
                        .await?;
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind(serde_json::to_string(&serde_json::json!("working"))?)
                        .bind(b"dirty-image-marker".as_slice())
                        .execute(candidate_pool.as_ref())
                        .await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .context("dirty candidate fixture deadline exceeded")??;
                let head: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                    .fetch_one(candidate_pool.as_ref())
                    .await?;
                let working: String = sqlx::query_scalar("SELECT DOLT_HASHOF_DB('WORKING')")
                    .fetch_one(candidate_pool.as_ref())
                    .await?;
                let staged: String = sqlx::query_scalar("SELECT DOLT_HASHOF_DB('STAGED')")
                    .fetch_one(candidate_pool.as_ref())
                    .await?;
                ensure!(working != staged, "dirty candidate roots were not distinct");
                let main = server.pool("main").await?;
                let usage = server.pool("kuru_usage_v1").await?;
                let usage_head: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                    .fetch_one(usage.as_ref())
                    .await?;
                sqlx::query("CALL DOLT_TAG(?, ?, '--author', ?)")
                    .bind("arbitrary-usage-head-alias")
                    .bind(&usage_head)
                    .bind(crate::store::AUTHOR)
                    .execute(main.as_ref())
                    .await?;
                sqlx::query("CALL DOLT_TAG(?, ?, '--author', ?)")
                    .bind("retained-backup-tag")
                    .bind(&head)
                    .bind(crate::store::AUTHOR)
                    .execute(main.as_ref())
                    .await?;
                // A native branch whose session has never been selected is
                // retained too; validation must not synthesize source commits.
                sqlx::query("CALL DOLT_BRANCH(?, ?)")
                    .bind("retained-unopened")
                    .bind(&head)
                    .execute(main.as_ref())
                    .await?;
                let (sql_instance, sql_scope) = server.sql_origin();
                // The ordinary temporary-store allowance is deliberately
                // unbound to a real project. This private inspector fixture
                // supplies its actual SQL origin, never production authority.
                let source = BackupSource {
                    project_path: crate::service::project_path_bytes(root.path()),
                    storage_scope: crate::store::temporary_scope(),
                    history_scope: crate::store::temporary_scope(),
                    store_instance: server.instance().to_owned(),
                    sql_origin_instance: sql_instance.to_owned(),
                    sql_origin_scope: sql_scope.to_owned(),
                };
                let image = root.path().join("image");
                crate::files::private_dir(&image)?;
                let cancelled = BackupCancellation::default();
                capture::copy(store.backup_connection()?, image.clone(), &cancelled).await?;
                let cut = pinned_nbs_root(&image, Privacy::OwnerOnly)?;
                let binary = crate::provision::provision(
                    &kuru_core::MemoryConfig::default(),
                    &root.path().join("engine-cache"),
                )
                .await?;
                let parent = crate::files::directory(root.path())?;
                let stage = native::ImageStage::new(
                    parent.create_private_directory(OsStr::new("validation"))?,
                    binary,
                    lifecycle_root.clone(),
                    None,
                )?;
                stage.restore(image.clone(), &cancelled).await?;
                stage.fsck(&cancelled).await?;
                let inspected = stage
                    .inspect(source, crate::store::test_supervisor()?, &cancelled)
                    .await?;
                ensure!(
                    inspected.schema_version == 10,
                    "copied main schema differs from capture"
                );
                let found = inspected
                    .refs
                    .iter()
                    .find(|reference| {
                        reference.kind == BackupRefKind::Branch && reference.name == candidate_name
                    })
                    .context("copied candidate ref missing")?;
                ensure!(
                    found.head == head
                        && found.working.as_ref().is_some_and(|roots| roots.dirty
                            && roots.working_root == working
                            && roots.staged_root == staged),
                    "copied dirty candidate roots differ from capture"
                );
                ensure!(
                    inspected
                        .refs
                        .iter()
                        .any(|reference| reference.kind == BackupRefKind::Tag
                            && reference.name == "retained-backup-tag"
                            && reference.head == head),
                    "copied tag differs from capture"
                );
                ensure!(
                    inspected
                        .refs
                        .iter()
                        .any(|reference| reference.kind == BackupRefKind::Branch
                            && reference.name == "kuru_usage_v1"),
                    "copied usage ref missing"
                );
                ensure!(
                    inspected
                        .refs
                        .iter()
                        .any(|reference| reference.kind == BackupRefKind::Tag
                            && reference.name == "arbitrary-usage-head-alias"
                            && reference.head == usage_head),
                    "copied arbitrary usage-head tag missing"
                );
                ensure!(
                    inspected
                        .refs
                        .iter()
                        .any(|reference| reference.kind == BackupRefKind::Branch
                            && reference.name == "retained-unopened"
                            && reference.head == head),
                    "copied unopened native branch missing"
                );
                ensure!(
                    pinned_nbs_root(&image, Privacy::OwnerOnly)? == cut,
                    "SQL validation changed original image"
                );
                let still_working: String = sqlx::query_scalar("SELECT DOLT_HASHOF_DB('WORKING')")
                    .fetch_one(candidate_pool.as_ref())
                    .await?;
                ensure!(
                    still_working == working,
                    "validation normalized source candidate"
                );
                drop(candidate_pool);
                drop(main);
                drop(usage);
                drop(candidate);
                store.close().await?;
                Ok(())
            }
            .await;
            let cleanup = if stage_path.exists() {
                crate::test_support::await_store_quiescence(&stage_path, lifecycle_root.as_deref())
                    .await
            } else {
                Ok(())
            };
            let outcome = match (outcome, cleanup) {
                (Ok(()), cleanup) => cleanup,
                (Err(error), Ok(())) => Err(error),
                (Err(error), Err(cleanup)) => Err(error.context(format!(
                    "SQL image fixture cleanup also failed: {cleanup:#}"
                ))),
            };
            root.release(outcome)
        })
        .await
    }

    #[tokio::test]
    async fn pinned_sql_working_and_staged_select_exact_distinct_roots() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let store = crate::store::MemoryStore::temporary_cold().await?;
            crate::test_support::closing::register(store.server_for_teardown());
            store
                .put("root-selector", &serde_json::json!("committed"))
                .await?;
            let pool = store.server_for_teardown().pool("main").await?;
            crate::pool::within(crate::store::QUERY_TIMEOUT, async {
                sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                    .bind(serde_json::to_string(&serde_json::json!("staged"))?)
                    .bind(b"root-selector".as_slice())
                    .execute(pool.as_ref())
                    .await?;
                sqlx::query("CALL DOLT_ADD(?)")
                    .bind("state")
                    .execute(pool.as_ref())
                    .await?;
                sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                    .bind(serde_json::to_string(&serde_json::json!("working"))?)
                    .bind(b"root-selector".as_slice())
                    .execute(pool.as_ref())
                    .await?;
                let mut roots = Vec::new();
                for (selector, statement, expected) in [
                    (
                        "HEAD",
                        "SELECT value FROM state AS OF 'HEAD' WHERE `key` = ?",
                        "committed",
                    ),
                    (
                        "STAGED",
                        "SELECT value FROM state AS OF 'STAGED' WHERE `key` = ?",
                        "staged",
                    ),
                    (
                        "WORKING",
                        "SELECT value FROM state AS OF 'WORKING' WHERE `key` = ?",
                        "working",
                    ),
                ] {
                    let root: String = sqlx::query_scalar("SELECT DOLT_HASHOF_DB(?)")
                        .bind(selector)
                        .fetch_one(pool.as_ref())
                        .await?;
                    ensure!(noms_hash(&root), "invalid pinned {selector} root");
                    roots.push(root);
                    // Only fixed native selectors become query text. No user
                    // ref or native root hash is treated as a commit name.
                    let value: String = sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
                        .bind(b"root-selector".as_slice())
                        .fetch_one(pool.as_ref())
                        .await?;
                    ensure!(
                        serde_json::from_str::<serde_json::Value>(&value)?
                            == serde_json::json!(expected),
                        "pinned AS OF {selector} selected the wrong retained value"
                    );
                }
                ensure!(
                    roots[0] != roots[1] && roots[1] != roots[2] && roots[0] != roots[2],
                    "pinned working/staged fixture did not establish distinct native roots"
                );
                let tables: Vec<String> = sqlx::query_scalar("SHOW TABLES AS OF 'STAGED'")
                    .fetch_all(pool.as_ref())
                    .await?;
                ensure!(
                    tables.iter().any(|table| table == "state"),
                    "staged table enumeration omitted state"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("pinned working/staged selector query budget expired")??;
            // Leave the real staged and working roots untouched: the native
            // selector proof must not depend on committing or normalizing them.
            drop(pool);
            store.close().await?;
            Ok(())
        })
        .await
    }

    #[test]
    fn cancellation_requires_an_actual_request_and_reaches_all_steps() {
        use std::future::Future;
        let cancellation = BackupCancellation::default();
        let next_step = cancellation.clone();
        let mut waiting = Box::pin(cancellation.cancelled());
        let waker = futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert!(waiting.as_mut().poll(&mut context).is_pending());
        assert!(!next_step.is_cancelled());
        cancellation.cancel();
        assert!(waiting.as_mut().poll(&mut context).is_ready());
        assert!(next_step.is_cancelled());
        assert!(
            Box::pin(next_step.cancelled())
                .as_mut()
                .poll(&mut context)
                .is_ready()
        );
    }

    #[test]
    fn pinned_manifest_root_is_exact_and_rejects_unknown_header() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let image = root.path().join("image");
        crate::files::private_dir(&image)?;
        let manifest = image.join("manifest");
        let root_hash = "f8onodnpbe24h7mi13eeih2drg735rb4";
        crate::files::write(
            &manifest,
            format!(
                "5:__DOLT__:{}:{root_hash}:{}:table:1",
                "0".repeat(32),
                "0".repeat(32)
            )
            .as_bytes(),
        )?;
        assert_eq!(pinned_nbs_root(&image, Privacy::OwnerOnly)?, root_hash);
        crate::files::write(
            &manifest,
            format!("6:__DOLT__:{}:{root_hash}:", "0".repeat(32)).as_bytes(),
        )?;
        assert!(pinned_nbs_root(&image, Privacy::OwnerOnly).is_err());
        Ok(())
    }

    #[test]
    fn manifest_checks_identity_inventory_and_native_provenance_separately() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let backup = root.path().join("backup");
        crate::files::private_dir(&backup)?;
        let image = backup.join("image");
        crate::files::private_dir(&image)?;
        let hash = "f8onodnpbe24h7mi13eeih2drg735rb4";
        crate::files::write(
            &image.join("manifest"),
            format!("5:__DOLT__:{}:{hash}:", "0".repeat(32)).as_bytes(),
        )?;
        let source_path = b"/canonical/source/project".to_vec();
        let digest = Sha256::digest(&source_path);
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let mut manifest = BackupManifest::new(
            BackupSource {
                project_path: source_path,
                storage_scope: scope.clone(),
                history_scope: scope.clone(),
                store_instance: uuid::Uuid::new_v4().to_string(),
                sql_origin_instance: uuid::Uuid::new_v4().to_string(),
                sql_origin_scope: scope,
            },
            9,
            vec![BackupRef {
                kind: BackupRefKind::Branch,
                name: "main".into(),
                head: hash.into(),
                working: Some(BackupWorkingSet {
                    working_root: hash.into(),
                    staged_root: hash.into(),
                    dirty: false,
                }),
            }],
            &image,
        )?;
        // Older engine/schema provenance is not a byte-format compatibility
        // decision. The separate native validator must establish that later.
        manifest.dolt_version = "2.3.3".into();
        manifest.validate_fields()?;
        manifest.verify_image(&image)?;
        let metadata = backup.join("backup.json");
        manifest.write(&metadata)?;
        assert_eq!(BackupManifest::read(&metadata)?, manifest);
        let checked = CheckedBackup::read(&backup)?;
        checked.verify_unchanged()?;
        assert_eq!(checked.manifest, manifest);
        let mut duplicate = manifest.clone();
        duplicate.files.push(duplicate.files[0].clone());
        assert!(duplicate.validate_fields().is_err());
        let mut duplicate_ref = manifest.clone();
        duplicate_ref.refs.push(duplicate_ref.refs[0].clone());
        assert!(duplicate_ref.validate_fields().is_err());
        let mut missing_working = manifest.clone();
        missing_working.refs[0].working = None;
        assert!(missing_working.validate_fields().is_err());
        let mut different_identity = manifest.clone();
        different_identity.source_storage_scope = format!("project/{}", "0".repeat(64));
        assert!(different_identity.validate_fields().is_err());
        crate::files::write(&image.join("manifest"), b"different image bytes")?;
        assert!(manifest.verify_image(&image).is_err());
        assert!(checked.verify_unchanged().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn native_output_drains_beyond_retained_bound_to_eof() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let (mut writer, reader) = tokio::io::duplex(1024);
            let bytes = 3 * BUFFER_BYTES + 17;
            let writing = tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let buffer = [b'x'; 4096];
                let mut remaining = bytes;
                while remaining != 0 {
                    let count = remaining.min(buffer.len());
                    writer.write_all(&buffer[..count]).await?;
                    remaining -= count;
                }
                writer.shutdown().await
            });
            let drained = drain_native_output(reader).await?;
            writing.await??;
            assert_eq!(drained.bytes, bytes as u64);
            assert_eq!(drained.retained, vec![b'x'; BUFFER_BYTES]);
            Ok(())
        })
        .await
    }
}
