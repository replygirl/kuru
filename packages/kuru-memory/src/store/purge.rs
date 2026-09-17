use super::*;
use std::io::Read;

const CONTROL_FORMAT: u32 = 1;
const MAX_TARGETS: usize = 64;

/// The completed outcome of an explicitly confirmed managed-project purge.
///
/// The operation removes Kuru's current Dolt tree and its managed recovery
/// trees for this exact project. It intentionally retains original/shared
/// legacy SQLite inputs, user exports, stable lock objects, and other projects.
#[derive(Clone, Debug, Serialize)]
pub struct PurgeOutcome {
    pub project: String,
    pub removed_trees: usize,
    pub legacy_import_suppressed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PurgeControl {
    format: u32,
    project_scope: String,
    suppress_legacy_import: bool,
    pending: Option<PendingPurge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingPurge {
    operation: Uuid,
    targets: Vec<PurgeTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PurgeTarget {
    source: SourceLocation,
    identity: [u8; 24],
    quarantine_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "location", rename_all = "snake_case", deny_unknown_fields)]
enum SourceLocation {
    Active,
    Staging { name: String },
    Interrupted { name: String },
}

impl MemoryStore {
    /// Explicitly remove this project's managed current and recovery Dolt
    /// trees. This never opens a store, provisions Dolt, imports legacy data,
    /// or contacts a provider. A durable control record suppresses future
    /// automatic import from a shared legacy SQLite source.
    pub async fn purge(options: OpenOptions) -> Result<PurgeOutcome> {
        options.config.validate()?;
        ensure!(
            !options.read_only,
            "cannot purge through a read-only memory inspection option"
        );
        let project = project_directory(&options.data_dir, &options.project_scope)?;
        private_dir(&options.data_dir)?;
        let memory = project.parent().context("project store has no parent")?;
        private_dir(memory)?;
        let name = project.file_name().context("project store has no name")?;
        let locks = memory.join("locks");
        private_dir(&locks)?;
        let lock_directory =
            files::open_directory(&locks, Privacy::OwnerOnly, NameRetention::Pinned)?;
        let startup_lock = acquire_lock(
            lock_directory.lock_file(name)?,
            Duration::from_secs(options.config.startup_timeout_secs),
        )
        .await?;
        lock_directory.verify(name, &startup_lock)?;

        let control_path = control_path(memory, &options.project_scope)?;
        let mut control = load_control(&control_path, &options.project_scope)?;
        let pending = match control.as_mut().and_then(|control| control.pending.take()) {
            Some(pending) => pending,
            None => {
                let operation = Uuid::new_v4();
                let targets = inventory_targets(memory, &options.project_scope, operation)?;
                let pending = PendingPurge { operation, targets };
                // Every target is checked and quiescent before publishing intent.
                let leases = acquire_all_leases(
                    memory,
                    &options.project_scope,
                    pending.operation,
                    &pending.targets,
                    Duration::from_secs(options.config.startup_timeout_secs),
                )
                .await?;
                let next = PurgeControl {
                    format: CONTROL_FORMAT,
                    project_scope: options.project_scope.clone(),
                    suppress_legacy_import: true,
                    pending: Some(pending.clone()),
                };
                write_control(&control_path, &next)?;
                // The record is durable before the first move/removal. Keep all
                // leases alive and continue directly from their verified roots.
                return complete_pending(memory, control_path, next, leases, startup_lock).await;
            }
        };

        let next = PurgeControl {
            format: CONTROL_FORMAT,
            project_scope: options.project_scope.clone(),
            suppress_legacy_import: true,
            pending: Some(pending.clone()),
        };
        // Existing intent is already durable. Do not rewrite it before the
        // retry has acquired every live target lease: a live-owner refusal must
        // leave the control record untouched.
        let leases = acquire_all_leases(
            memory,
            &options.project_scope,
            pending.operation,
            &pending.targets,
            Duration::from_secs(options.config.startup_timeout_secs),
        )
        .await?;
        complete_pending(memory, control_path, next, leases, startup_lock).await
    }

    pub(crate) fn legacy_import_suppressed(data_dir: &Path, project_scope: &str) -> Result<bool> {
        let project = project_directory(data_dir, project_scope)?;
        let memory = project.parent().context("project store has no parent")?;
        Ok(
            load_control(&control_path(memory, project_scope)?, project_scope)?
                .is_some_and(|control| control.suppress_legacy_import),
        )
    }
}

pub(super) fn ensure_open_allowed(data_dir: &Path, project_scope: &str) -> Result<()> {
    let project = project_directory(data_dir, project_scope)?;
    let memory = project.parent().context("project store has no parent")?;
    if load_control(&control_path(memory, project_scope)?, project_scope)?
        .is_some_and(|control| control.pending.is_some())
    {
        bail!(
            "project purge is incomplete; resume `kuru memory purge --yes` before opening memory"
        );
    }
    Ok(())
}

async fn complete_pending(
    memory: &Path,
    control_path: PathBuf,
    mut control: PurgeControl,
    leases: Vec<PurgeLease>,
    _startup_lock: File,
) -> Result<PurgeOutcome> {
    let pending = control
        .pending
        .clone()
        .context("missing purge pending inventory")?;
    ensure!(
        leases.len() == pending.targets.len(),
        "purge lease inventory does not match durable control record"
    );
    let quarantine = purge_directory(memory)?;
    private_dir(&quarantine)?;
    let mut removed = 0;
    for lease in leases {
        match lease {
            PurgeLease::Absent => {}
            PurgeLease::Source { target, mut lease } => {
                let destination = quarantine.join(&target.quarantine_name);
                ensure_absent(
                    &destination,
                    "purge quarantine destination already exists; preserve and inspect it",
                )?;
                lease.move_to(&destination).with_context(|| {
                    format!("move managed project tree into purge quarantine: {destination:?}")
                })?;
                ensure!(
                    lease.directory.identity().to_bytes() == target.identity,
                    "moved purge target identity changed"
                );
                lease.remove_tree().with_context(|| {
                    format!("remove quarantined managed project tree: {destination:?}")
                })?;
                removed += 1;
            }
            PurgeLease::Quarantine { target, lease } => {
                let destination = quarantine.join(&target.quarantine_name);
                lease.remove_tree().with_context(|| {
                    format!("remove quarantined managed project tree: {destination:?}")
                })?;
                removed += 1;
            }
        }
    }
    control.pending = None;
    write_control(&control_path, &control)?;
    Ok(PurgeOutcome {
        project: control.project_scope,
        removed_trees: removed,
        legacy_import_suppressed: true,
    })
}

enum PurgeLease {
    Source {
        target: PurgeTarget,
        lease: LifecycleLease,
    },
    Quarantine {
        target: PurgeTarget,
        lease: LifecycleLease,
    },
    Absent,
}

async fn acquire_all_leases(
    memory: &Path,
    scope: &str,
    operation: Uuid,
    targets: &[PurgeTarget],
    timeout: Duration,
) -> Result<Vec<PurgeLease>> {
    ensure!(
        targets.len() <= MAX_TARGETS,
        "too many managed purge targets"
    );
    let lifecycle_root = cfg!(windows).then(|| memory.join("lifecycles"));
    let mut leases = Vec::with_capacity(targets.len());
    let quarantine = purge_directory(memory)?;
    for target in targets {
        validate_quarantine_name(target, operation, scope, targets)?;
        let source = source_path(memory, scope, &target.source)?;
        let source_directory = checked_target(&source, target.identity)?;
        let quarantine_directory =
            checked_target(&quarantine.join(&target.quarantine_name), target.identity)?;
        let (directory, location) = match (source_directory, quarantine_directory) {
            (Some(_), Some(_)) => {
                bail!("purge source and quarantine are both occupied; preserve both identities")
            }
            (Some(directory), None) => (directory, true),
            (None, Some(directory)) => (directory, false),
            (None, None) => {
                leases.push(PurgeLease::Absent);
                continue;
            }
        };
        let lease =
            Server::quiescence_at(directory.path(), lifecycle_root.as_deref(), timeout).await?;
        ensure!(
            lease.directory.identity().to_bytes() == target.identity,
            "purge lifecycle target identity changed"
        );
        leases.push(if location {
            PurgeLease::Source {
                target: target.clone(),
                lease,
            }
        } else {
            PurgeLease::Quarantine {
                target: target.clone(),
                lease,
            }
        });
    }
    Ok(leases)
}

fn inventory_targets(memory: &Path, scope: &str, operation: Uuid) -> Result<Vec<PurgeTarget>> {
    let project = project_directory(memory.parent().context("memory has no data parent")?, scope)?;
    let hash = project
        .file_name()
        .context("project store has no name")?
        .to_string_lossy();
    let mut locations = Vec::new();
    match fs::symlink_metadata(&project) {
        Ok(_) => {
            let directory = files::directory(&project)?;
            read_activation_checked(&directory, scope)?;
            locations.push((SourceLocation::Active, directory.identity().to_bytes()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    inventory_staging(memory, &hash, scope, false, &mut locations)?;
    let interrupted = memory.join("interrupted");
    match fs::symlink_metadata(&interrupted) {
        Ok(_) => {
            files::directory(&interrupted)?;
            inventory_staging(&interrupted, &hash, scope, true, &mut locations)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        locations.len() <= MAX_TARGETS,
        "too many managed purge targets"
    );
    locations
        .into_iter()
        .enumerate()
        .map(|(index, (source, identity))| {
            Ok(PurgeTarget {
                source,
                identity,
                quarantine_name: format!("{hash}.purge-{operation}-{index}"),
            })
        })
        .collect()
}

fn inventory_staging(
    parent: &Path,
    hash: &str,
    scope: &str,
    interrupted: bool,
    locations: &mut Vec<(SourceLocation, [u8; 24])>,
) -> Result<()> {
    let prefix = format!("{hash}.staging-");
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        let Some(suffix) = text.strip_prefix(&prefix) else {
            continue;
        };
        ensure!(
            Uuid::parse_str(suffix).is_ok() && entry.file_type()?.is_dir(),
            "unrecognized project staging path prevents an explicit purge: {}",
            entry.path().display()
        );
        let path = entry.path();
        match fs::symlink_metadata(path.join("ready.json")) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
                "unverified interrupted project staging prevents an explicit purge: {}",
                path.display()
            ),
            Err(error) => return Err(error.into()),
        }
        let directory = files::directory(&path)?;
        read_activation_checked(&directory, scope)?;
        locations.push((
            if interrupted {
                SourceLocation::Interrupted {
                    name: text.to_owned(),
                }
            } else {
                SourceLocation::Staging {
                    name: text.to_owned(),
                }
            },
            directory.identity().to_bytes(),
        ));
        ensure!(
            locations.len() <= MAX_TARGETS,
            "too many managed purge targets"
        );
    }
    Ok(())
}

fn source_path(memory: &Path, scope: &str, source: &SourceLocation) -> Result<PathBuf> {
    let hash = project_directory(memory.parent().context("memory has no data parent")?, scope)?
        .file_name()
        .context("project store has no name")?
        .to_string_lossy()
        .into_owned();
    match source {
        SourceLocation::Active => Ok(memory.join(&hash)),
        SourceLocation::Staging { name } => {
            validate_stage_name(name, &hash)?;
            Ok(memory.join(name))
        }
        SourceLocation::Interrupted { name } => {
            validate_stage_name(name, &hash)?;
            Ok(memory.join("interrupted").join(name))
        }
    }
}

fn validate_stage_name(name: &str, hash: &str) -> Result<()> {
    let suffix = name
        .strip_prefix(&format!("{hash}.staging-"))
        .context("purge control staging name is outside the exact project namespace")?;
    ensure!(
        Uuid::parse_str(suffix).is_ok(),
        "invalid purge control staging UUID"
    );
    Ok(())
}

fn validate_quarantine_name(
    target: &PurgeTarget,
    operation: Uuid,
    scope: &str,
    targets: &[PurgeTarget],
) -> Result<()> {
    let hash = scope
        .strip_prefix("project/")
        .context("memory project scope must start with project/")?;
    let index = targets
        .iter()
        .position(|candidate| candidate.quarantine_name == target.quarantine_name)
        .context("purge target is absent from its durable inventory")?;
    ensure!(
        target.quarantine_name == format!("{hash}.purge-{operation}-{index}"),
        "invalid purge quarantine name"
    );
    Ok(())
}

fn checked_target(path: &Path, expected: [u8; 24]) -> Result<Option<Directory>> {
    match files::open_directory(path, Privacy::OwnerOnly, NameRetention::Movable) {
        Ok(directory) => {
            ensure!(
                directory.identity().to_bytes() == expected,
                "purge target path has a replacement identity"
            );
            Ok(Some(directory))
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_activation_checked(directory: &Directory, scope: &str) -> Result<Activation> {
    directory.revalidate()?;
    let mut marker = directory.read(std::ffi::OsStr::new("ready.json")).context(
        "project memory has no activation record; preserve the store and repair it before opening",
    )?;
    let mut bytes = Vec::new();
    (&mut marker).take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024,
        "project memory activation record exceeds limit"
    );
    directory.verify(std::ffi::OsStr::new("ready.json"), &marker)?;
    directory.revalidate()?;
    let activation: Activation = serde_json::from_slice(&bytes)?;
    ensure!(
        activation.format == 1 && activation.project_scope == scope,
        "project memory activation identity does not match"
    );
    Ok(activation)
}

fn ensure_absent(path: &Path, message: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => bail!("{message}"),
        Err(error) => Err(error.into()),
    }
}

fn control_path(memory: &Path, scope: &str) -> Result<PathBuf> {
    let project = project_directory(memory.parent().context("memory has no data parent")?, scope)?;
    let hash = project.file_name().context("project store has no name")?;
    Ok(memory
        .join("controls")
        .join(format!("{}.json", hash.to_string_lossy())))
}

fn purge_directory(memory: &Path) -> Result<PathBuf> {
    Ok(memory.join("purged"))
}

fn load_control(path: &Path, scope: &str) -> Result<Option<PurgeControl>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let control: PurgeControl = serde_json::from_slice(&files::read_bytes(path, 32 * 1024)?)?;
    ensure!(
        control.format == CONTROL_FORMAT
            && control.project_scope == scope
            && control.suppress_legacy_import,
        "project purge control identity is invalid"
    );
    if let Some(pending) = &control.pending {
        ensure!(
            pending.targets.len() <= MAX_TARGETS,
            "project purge control has too many targets"
        );
        let memory = path
            .parent()
            .and_then(Path::parent)
            .context("purge control has no memory parent")?;
        let mut sources = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut quarantines = BTreeSet::new();
        for target in &pending.targets {
            let source = source_path(memory, scope, &target.source)?;
            ensure!(
                sources.insert(source),
                "purge control repeats a source target"
            );
            ensure!(
                identities.insert(target.identity),
                "purge control repeats a target physical identity"
            );
            ensure!(
                quarantines.insert(target.quarantine_name.clone()),
                "purge control repeats a quarantine target"
            );
            validate_quarantine_name(target, pending.operation, scope, &pending.targets)?;
        }
    }
    Ok(Some(control))
}

fn write_control(path: &Path, control: &PurgeControl) -> Result<()> {
    let parent = path.parent().context("purge control has no parent")?;
    private_dir(parent)?;
    files::write(path, &serde_json::to_vec(control)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as SqliteConnection;
    use std::io::Write;

    #[tokio::test]
    async fn explicit_purge_removes_one_project_history_and_suppresses_legacy_reimport()
    -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "d".repeat(64));
        let other_scope = format!("project/{}", "e".repeat(64));
        let legacy_path = root.path().join("memory.sqlite3");
        let source = SqliteConnection::open(&legacy_path)?;
        source.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             PRAGMA application_id=1263882837;
             PRAGMA user_version=1;
             CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
             CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )?;
        source.pragma_update(None, "application_id", 0x4b55_5255_i64)?;
        let key = format!("{scope}/preferences");
        source.execute(
            "INSERT INTO state VALUES (?1, '{\"mode\":\"ifs\"}')",
            [&key],
        )?;
        let legacy_bytes = fs::read(&legacy_path)?;

        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        let store = MemoryStore::open(options.clone()).await?;
        assert!(store.get(&key).await?.is_some());
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("purged-session").await?;
        ledger
            .admit(kuru_core::InvocationStart {
                session_id: "purged-session".into(),
                invocation_id: "purged-invocation".into(),
                operation_id: "purge-fixture".into(),
                phase: kuru_core::UsagePhase::Speak,
                actor_id: "fixture".into(),
                route: "fixture".into(),
                model: "fixture".into(),
                price_at_invocation: None,
            })
            .await?;
        drop(ledger);
        let revision = store.revision().await?;
        store.close().await?;
        let project = project_directory(root.path(), &scope)?;
        assert!(project.join("ready.json").is_file());

        let other = crate::test_support::open_options(root.path().to_owned(), other_scope.clone())?;
        MemoryStore::open(other.clone()).await?.close().await?;
        let purged = Directory::ensure_private(&root.path().join("memory/purged"))?;
        purged
            .create_new(std::ffi::OsStr::new("unrelated-sentinel"))?
            .write_all(b"retain this other purge record")?;

        let outcome = MemoryStore::purge(options.clone()).await?;
        assert_eq!(outcome.project, scope);
        assert_eq!(outcome.removed_trees, 1);
        assert!(outcome.legacy_import_suppressed);
        assert!(!project.exists());
        assert!(MemoryStore::exists(root.path(), &other_scope)?);
        assert_eq!(
            fs::read(root.path().join("memory/purged/unrelated-sentinel"))?,
            b"retain this other purge record"
        );
        assert_eq!(fs::read(&legacy_path)?, legacy_bytes);
        assert!(MemoryStore::legacy_import_suppressed(root.path(), &scope)?);

        let reopened = MemoryStore::open(options).await?;
        assert_ne!(reopened.revision().await?, revision);
        assert_eq!(reopened.get(&key).await?, None);
        assert_eq!(
            reopened
                .usage_ledger()?
                .session("purged-session")
                .await?
                .invocation_count,
            0
        );
        reopened.close().await?;
        drop(source);
        Ok(())
    }

    #[tokio::test]
    async fn explicit_purge_removes_verified_active_staging_and_interrupted_trees_only()
    -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "9".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        MemoryStore::open(options.clone()).await?.close().await?;

        let project = project_directory(root.path(), &scope)?;
        let memory = project.parent().unwrap();
        let hash = project.file_name().unwrap().to_string_lossy();
        let activation = fs::read(project.join("ready.json"))?;
        let staging = memory.join(format!("{hash}.staging-{}", Uuid::new_v4()));
        let staging_directory = Directory::ensure_private(&staging)?;
        staging_directory
            .create_new(std::ffi::OsStr::new("ready.json"))?
            .write_all(&activation)?;
        drop(staging_directory);
        let interrupted = memory
            .join("interrupted")
            .join(format!("{hash}.staging-{}", Uuid::new_v4()));
        let interrupted_directory = Directory::ensure_private(&interrupted)?;
        interrupted_directory
            .create_new(std::ffi::OsStr::new("ready.json"))?
            .write_all(&activation)?;
        drop(interrupted_directory);
        let other = memory.join(format!("{}.staging-{}", "8".repeat(64), Uuid::new_v4()));
        let other_directory = Directory::ensure_private(&other)?;
        other_directory
            .create_new(std::ffi::OsStr::new("sentinel"))?
            .write_all(b"other project staging survives")?;
        drop(other_directory);

        let outcome = MemoryStore::purge(options).await?;
        assert_eq!(outcome.removed_trees, 3);
        assert!(!project.exists());
        assert!(!staging.exists());
        assert!(!interrupted.exists());
        assert_eq!(
            fs::read(other.join("sentinel"))?,
            b"other project staging survives"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unverified_same_project_staging_refuses_before_publishing_or_removing() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "7".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        MemoryStore::open(options.clone()).await?.close().await?;
        let project = project_directory(root.path(), &scope)?;
        let memory = project.parent().unwrap();
        let hash = project.file_name().unwrap().to_string_lossy();
        let staging = memory.join(format!("{hash}.staging-{}", Uuid::new_v4()));
        let staging_directory = Directory::ensure_private(&staging)?;
        staging_directory
            .create_new(std::ffi::OsStr::new("sentinel"))?
            .write_all(b"unverified staging is retained")?;
        drop(staging_directory);

        let error = MemoryStore::purge(options).await.unwrap_err();
        assert!(format!("{error:#}").contains("unverified interrupted project staging"));
        assert!(project.join("ready.json").is_file());
        assert_eq!(
            fs::read(staging.join("sentinel"))?,
            b"unverified staging is retained"
        );
        assert!(fs::symlink_metadata(control_path(memory, &scope)?).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn live_owner_refusal_leaves_no_purge_control_or_tree_change() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "f".repeat(64));
        let mut options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        let store = MemoryStore::open(options.clone()).await?;
        let project = project_directory(root.path(), &scope)?;
        let revision = store.revision().await?;

        options.config.startup_timeout_secs = 1;
        let error = MemoryStore::purge(options).await.unwrap_err();
        assert!(format!("{error:#}").contains("memory lifecycle remains active"));
        assert_eq!(store.revision().await?, revision);
        assert!(project.join("ready.json").is_file());
        assert!(
            fs::symlink_metadata(control_path(project.parent().unwrap(), &scope)?).is_err(),
            "live-owner refusal must not publish durable purge intent"
        );
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn read_only_options_cannot_request_a_destructive_purge() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "c".repeat(64));
        let mut options = OpenOptions::new(root.path().to_owned(), scope);
        options.read_only = true;
        let error = MemoryStore::purge(options).await.unwrap_err();
        assert!(format!("{error:#}").contains("read-only memory inspection"));
        assert!(!root.path().join("memory").exists());
        Ok(())
    }

    #[tokio::test]
    async fn reconstructed_pending_purge_blocks_open_then_resumes_only_its_recorded_tree()
    -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "b".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        MemoryStore::open(options.clone()).await?.close().await?;
        let project = project_directory(root.path(), &scope)?;
        let memory = project.parent().unwrap();
        let operation = Uuid::new_v4();
        let target = PurgeTarget {
            source: SourceLocation::Active,
            identity: files::directory(&project)?.identity().to_bytes(),
            quarantine_name: format!("{}.purge-{operation}-0", "b".repeat(64)),
        };
        let path = control_path(memory, &scope)?;
        write_control(
            &path,
            &PurgeControl {
                format: CONTROL_FORMAT,
                project_scope: scope.clone(),
                suppress_legacy_import: true,
                pending: Some(PendingPurge {
                    operation,
                    targets: vec![target],
                }),
            },
        )?;
        let blocked = MemoryStore::open(options.clone()).await.unwrap_err();
        assert!(format!("{blocked:#}").contains("purge is incomplete"));

        let outcome = MemoryStore::purge(options).await?;
        assert_eq!(outcome.removed_trees, 1);
        assert!(!project.exists());
        assert!(load_control(&path, &scope)?.unwrap().pending.is_none());

        // Reconstruct the later durable state separately: the original target
        // was already moved into its exact quarantine name and its activation
        // marker is gone. A retry must still remove that recorded physical
        // tree; it must not require a marker or rediscover a new active path.
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        MemoryStore::open(options.clone()).await?.close().await?;
        let operation = Uuid::new_v4();
        let quarantined =
            purge_directory(memory)?.join(format!("{}.purge-{operation}-0", "b".repeat(64)));
        let identity = files::directory(&project)?.identity().to_bytes();
        fs::rename(&project, &quarantined)?;
        fs::remove_file(quarantined.join("ready.json"))?;
        write_control(
            &path,
            &PurgeControl {
                format: CONTROL_FORMAT,
                project_scope: scope.clone(),
                suppress_legacy_import: true,
                pending: Some(PendingPurge {
                    operation,
                    targets: vec![PurgeTarget {
                        source: SourceLocation::Active,
                        identity,
                        quarantine_name: format!("{}.purge-{operation}-0", "b".repeat(64)),
                    }],
                }),
            },
        )?;
        let blocked = MemoryStore::open(options.clone()).await.unwrap_err();
        assert!(format!("{blocked:#}").contains("purge is incomplete"));

        let outcome = MemoryStore::purge(options).await?;
        assert_eq!(outcome.removed_trees, 1);
        assert!(!quarantined.exists());
        assert!(load_control(&path, &scope)?.unwrap().pending.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn pending_purge_refuses_a_replacement_root_without_deleting_it() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "a".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        MemoryStore::open(options.clone()).await?.close().await?;
        let project = project_directory(root.path(), &scope)?;
        let memory = project.parent().unwrap();
        let operation = Uuid::new_v4();
        let target = PurgeTarget {
            source: SourceLocation::Active,
            identity: files::directory(&project)?.identity().to_bytes(),
            quarantine_name: format!("{}.purge-{operation}-0", "a".repeat(64)),
        };
        let path = control_path(memory, &scope)?;
        write_control(
            &path,
            &PurgeControl {
                format: CONTROL_FORMAT,
                project_scope: scope.clone(),
                suppress_legacy_import: true,
                pending: Some(PendingPurge {
                    operation,
                    targets: vec![target],
                }),
            },
        )?;
        let parked = root.path().join("parked-original");
        std::fs::rename(&project, &parked)?;
        let replacement = Directory::ensure_private(&project)?;
        replacement
            .create_new(std::ffi::OsStr::new("replacement"))?
            .write_all(b"retain replacement")?;

        let error = MemoryStore::purge(options).await.unwrap_err();
        assert!(format!("{error:#}").contains("replacement identity"));
        assert_eq!(
            fs::read(project.join("replacement"))?,
            b"retain replacement"
        );
        assert!(parked.join("ready.json").is_file());
        assert!(load_control(&path, &scope)?.unwrap().pending.is_some());
        Ok(())
    }
}
