//! One-way, read-only import. The original and a consistent full snapshot survive.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::files;
use crate::store::{identifier, private_dir, private_file, project_directory};
#[cfg(windows)]
use anyhow::Error;
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub(crate) struct LegacyImport {
    pub receipt: MigrationReceipt,
    pub messages: Vec<LegacyMessage>,
    pub state: Vec<(String, String)>,
}

#[derive(Debug)]
pub(crate) struct LegacyMessage {
    pub sequence: i64,
    pub namespace: String,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MigrationReceipt {
    pub source_sha256: String,
    pub snapshot: PathBuf,
    pub project_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_project_scope: Option<String>,
    pub messages: usize,
    pub state: usize,
}

/// Definite no-effect refusal of one explicit legacy import. Each variant is
/// proved before the selected project is published.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyImportRefusal {
    /// The destination project already has active memory.
    AlreadyActive,
    /// Explicit purge suppression retains the destination's absence.
    Suppressed,
    /// Another client remains attached to the live project service.
    ActiveClients,
    /// No legacy source or no row proves the selected source scope.
    SourceScopeUnproved,
    /// The selected source rows changed between admission and snapshot.
    SourceChanged,
    /// A row extends the selected scope without its exact separator.
    AmbiguousSourceScope,
    /// The legacy source already contains destination-scoped identities.
    DestinationCollision,
    /// A moved import found a state family that cannot be proved safe to remap.
    UnsupportedMovedState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyImportRejected(pub LegacyImportRefusal);

impl std::fmt::Display for LegacyImportRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.0 {
            LegacyImportRefusal::AlreadyActive => {
                "project memory is already active; explicit legacy import cannot overwrite it"
            }
            LegacyImportRefusal::Suppressed => {
                "legacy import was explicitly suppressed for this project"
            }
            LegacyImportRefusal::ActiveClients => {
                "memory service has active clients; close them before explicit legacy import"
            }
            LegacyImportRefusal::SourceScopeUnproved => {
                "selected legacy source scope has no proved rows"
            }
            LegacyImportRefusal::SourceChanged => {
                "selected legacy source scope changed before import"
            }
            LegacyImportRefusal::AmbiguousSourceScope => {
                "legacy source has an ambiguous selected scope"
            }
            LegacyImportRefusal::DestinationCollision => {
                "legacy destination identity already occurs in source"
            }
            LegacyImportRefusal::UnsupportedMovedState => {
                "legacy state has an unsupported family for moved-project import"
            }
        };
        formatter.write_str(reason)
    }
}

impl std::error::Error for LegacyImportRejected {}

/// How long the legacy snapshot may go without progress while its source is
/// busy or locked. Progress restarts the bound, so large valid copies can
/// complete however long the total operation takes.
const SNAPSHOT_STALL_BOUND: Duration = Duration::from_secs(30);

/// Drive backup steps until `Done`, failing only after a full no-progress
/// interval. This remains shared by automatic and explicit imports.
fn run_backup(
    mut step: impl FnMut() -> rusqlite::Result<StepResult>,
    mut now: impl FnMut() -> Instant,
) -> Result<()> {
    let mut deadline = now() + SNAPSHOT_STALL_BOUND;
    loop {
        ensure!(
            now() < deadline,
            "legacy memory snapshot made no progress for {} s while the source was busy or locked; retry when it is free",
            SNAPSHOT_STALL_BOUND.as_secs()
        );
        match step()? {
            StepResult::Done => return Ok(()),
            StepResult::More => deadline = now() + SNAPSHOT_STALL_BOUND,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Definite refusal of a bounded legacy inventory; no partial list is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyInventoryRefusal {
    /// The source holds more rows than one inventory observes.
    RowLimit,
    /// A namespace or state key exceeds the inventory's byte bound.
    NameLimit,
    /// The source names more project scopes than one inventory lists.
    ScopeLimit,
    /// The consistent read view did not finish within its interval.
    Deadline,
}

impl std::fmt::Display for LegacyInventoryRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::RowLimit => "legacy inventory row limit exceeded",
            Self::NameLimit => "legacy inventory name byte limit exceeded",
            Self::ScopeLimit => "legacy inventory scope limit exceeded",
            Self::Deadline => "legacy inventory deadline exceeded",
        })
    }
}

impl std::error::Error for LegacyInventoryRefusal {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InventoryScope {
    pub scope: String,
    pub messages: u64,
    pub state: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyInventory {
    pub scopes: Vec<InventoryScope>,
    pub unresolved_rows: u64,
}

/// Inspect one consistent legacy SQLite read view without creating a Dolt
/// project or a retained migration snapshot. SQLite may rebuild its SHM index.
pub(crate) fn inventory(data_dir: &Path) -> Result<Option<LegacyInventory>> {
    inventory_with_deadline(data_dir, Instant::now() + INVENTORY_TIMEOUT)
}

/// Bounded observation interval of one legacy inventory read view.
pub(crate) const INVENTORY_TIMEOUT: Duration = Duration::from_secs(30);

fn inventory_with_deadline(data_dir: &Path, deadline: Instant) -> Result<Option<LegacyInventory>> {
    inventory_read(data_dir, deadline).map_err(|error| {
        if interrupted(&error) && Instant::now() >= deadline {
            anyhow::Error::new(LegacyInventoryRefusal::Deadline)
        } else {
            error
        }
    })
}

fn interrupted(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<rusqlite::Error>())
        .any(|cause| cause.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted))
}

/// Pin the original source and sidecars and open one read-only, deadline
/// interrupted connection. An absent source is `None`.
fn open_source(data_dir: &Path, deadline: Instant) -> Result<Option<(SourcePins, Connection)>> {
    let source = data_dir.join("memory.sqlite3");
    match fs::symlink_metadata(&source) {
        Ok(metadata) => ensure!(metadata.is_file(), "legacy memory must be a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let pins = SourcePins::new(data_dir)?;
    let source = pins.parent.path().join("memory.sqlite3");
    let source = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("cannot inspect legacy SQLite memory")?;
    source.busy_timeout(Duration::from_secs(5))?;
    source.progress_handler(1_000, Some(move || Instant::now() >= deadline))?;
    Ok(Some((pins, source)))
}

fn inventory_read(data_dir: &Path, deadline: Instant) -> Result<Option<LegacyInventory>> {
    const MAX_ROWS: u64 = 250_000;
    const MAX_SCOPES: usize = 1_024;
    const MAX_NAME_BYTES: i64 = 1_024;

    let Some((pins, source)) = open_source(data_dir, deadline)? else {
        return Ok(None);
    };
    source.execute_batch("BEGIN")?;
    validate(&source)?;
    let mut scopes = BTreeMap::<String, (u64, u64)>::new();
    let mut unresolved_rows = 0u64;
    let mut total_rows = 0u64;
    for (table, column, is_message) in [("messages", "namespace", true), ("state", "key", false)] {
        let query = format!("SELECT length({column}), {column} FROM {table}");
        let mut statement = source.prepare(&query)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            total_rows += 1;
            ensure!(total_rows <= MAX_ROWS, LegacyInventoryRefusal::RowLimit);
            ensure!(Instant::now() < deadline, LegacyInventoryRefusal::Deadline);
            let length: i64 = row.get(0)?;
            ensure!(
                (0..=MAX_NAME_BYTES).contains(&length),
                LegacyInventoryRefusal::NameLimit
            );
            let name: String = row.get(1)?;
            ensure!(
                name.len() <= MAX_NAME_BYTES as usize,
                LegacyInventoryRefusal::NameLimit
            );
            if let Some(scope) = inventory_scope(&name) {
                ensure!(
                    scopes.contains_key(scope) || scopes.len() < MAX_SCOPES,
                    LegacyInventoryRefusal::ScopeLimit
                );
                let entry = scopes.entry(scope.to_owned()).or_default();
                if is_message {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                }
            } else {
                unresolved_rows += 1;
            }
        }
    }
    source.execute_batch("COMMIT")?;
    drop(source);
    pins.verify()?;
    Ok(Some(LegacyInventory {
        scopes: scopes
            .into_iter()
            .map(|(scope, (messages, state))| InventoryScope {
                scope,
                messages,
                state,
            })
            .collect(),
        unresolved_rows,
    }))
}

fn inventory_scope(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("project/")?;
    let digest = rest.get(..64)?;
    let suffix = rest.get(64..)?;
    if !digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || !suffix.starts_with('/')
        || suffix.len() == 1
    {
        return None;
    }
    name.get(.."project/".len() + 64)
}

/// Prove that one source scope has at least one row, stopping at the first
/// match. Unlike inventory, no whole-source row, name or scope bound applies;
/// the observation interval still bounds the read view.
pub(crate) fn source_scope_proved(data_dir: &Path, source_scope: &str) -> Result<bool> {
    let deadline = Instant::now() + INVENTORY_TIMEOUT;
    let Some((pins, source)) = open_source(data_dir, deadline)? else {
        return Ok(false);
    };
    // Under BINARY collation, every `scope/<non-empty>` name sorts after
    // `scope/` and before `scope0` ('/' precedes '0'); nothing else does.
    let lower = format!("{source_scope}/");
    let upper = format!("{source_scope}0");
    let proved = (|| -> Result<bool> {
        source.execute_batch("BEGIN")?;
        validate_identity(&source)?;
        let proved = source.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE namespace > ?1 AND namespace < ?2)
                 OR EXISTS(SELECT 1 FROM state WHERE key > ?1 AND key < ?2)",
            rusqlite::params![lower, upper],
            |row| row.get::<_, bool>(0),
        )?;
        source.execute_batch("COMMIT")?;
        Ok(proved)
    })();
    drop(source);
    let proved = proved.map_err(|error| {
        if interrupted(&error) && Instant::now() >= deadline {
            error.context(
                "legacy source scope proof deadline exceeded; retry when older writers are idle",
            )
        } else {
            error
        }
    })?;
    pins.verify()?;
    Ok(proved)
}

/// Row selection of one legacy import. Automatic first-open import keeps its
/// established exact-prefix rule; explicit import also refuses a row that
/// extends the selected scope without its separator.
#[derive(Clone, Copy)]
struct Selection<'a> {
    project_scope: &'a str,
    source_scope: &'a str,
    explicit: bool,
}

impl Selection<'_> {
    fn changed(&self) -> bool {
        self.project_scope != self.source_scope
    }

    /// The name's suffix after the selected scope, or `None` for a row outside
    /// it. Every refusal is decided from the name alone.
    fn select<'n>(&self, name: &'n str, is_state: bool) -> Result<Option<&'n str>> {
        let extended = name.strip_prefix(self.source_scope);
        let selected = extended.and_then(|rest| rest.strip_prefix('/'));
        ensure!(
            !self.explicit || extended.is_none() || selected.is_some(),
            LegacyImportRejected(LegacyImportRefusal::AmbiguousSourceScope)
        );
        ensure!(
            !(self.changed()
                && name
                    .strip_prefix(self.project_scope)
                    .is_some_and(|rest| rest.starts_with('/'))),
            LegacyImportRejected(LegacyImportRefusal::DestinationCollision)
        );
        let Some(suffix) = selected else {
            return Ok(None);
        };
        if self.changed() && is_state {
            ensure!(
                supported_moved_state_family(suffix),
                LegacyImportRejected(LegacyImportRefusal::UnsupportedMovedState)
            );
        }
        Ok(Some(suffix))
    }
}

/// Automatic first-open import of the exact canonical scope.
pub(crate) fn prepare(data_dir: &Path, project_scope: &str) -> Result<Option<LegacyImport>> {
    prepare_selected(
        data_dir,
        Selection {
            project_scope,
            source_scope: project_scope,
            explicit: false,
        },
    )
}

/// Explicit import of one selected source scope into `project_scope`. Every
/// definite refusal leaves no candidate or new snapshot under `memory/legacy`.
pub(crate) fn prepare_explicit(
    data_dir: &Path,
    project_scope: &str,
    source_scope: &str,
) -> Result<Option<LegacyImport>> {
    prepare_selected(
        data_dir,
        Selection {
            project_scope,
            source_scope,
            explicit: true,
        },
    )
}

fn prepare_selected(data_dir: &Path, selection: Selection<'_>) -> Result<Option<LegacyImport>> {
    project_directory(data_dir, selection.project_scope)?;
    project_directory(data_dir, selection.source_scope)?;
    let source = data_dir.join("memory.sqlite3");
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file(),
        "legacy memory must be a regular file, not a link"
    );
    // Checked pinned handles protect names while SQLite's backup API supplies
    // consistency with concurrent committed WAL writers. Normal SHM rebuilds
    // are coordination, not a promise of byte-identical shared-memory indexes.
    let pins = SourcePins::new(data_dir)?;
    let source = pins.parent.path().join("memory.sqlite3");
    let source = Connection::open_with_flags(
        &source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("cannot open legacy SQLite memory read-only")?;
    source.busy_timeout(Duration::from_secs(5))?;
    if selection.explicit {
        // Decide explicit refusals from names before copying any private row.
        validate(&source)?;
        ensure!(
            precheck(&source, selection)?,
            LegacyImportRejected(LegacyImportRefusal::SourceChanged)
        );
    }
    let snapshots = data_dir.join("memory/legacy");
    private_dir(&snapshots)?;
    let snapshot_directory = files::directory(&snapshots)?;
    let candidate = snapshots.join(format!("{}.sqlite3", uuid::Uuid::new_v4()));
    let candidate_name = files::name(&candidate)?;
    let file = private_file(&candidate)?;
    // The candidate is unpublished until this point succeeds; any failure
    // removes it through the checked directory instead of retaining a copy.
    let checked = fill_candidate(source, &pins, &candidate, &file, selection).and_then(|rows| {
        snapshot_directory.verify(candidate_name, &file)?;
        let source_sha256 = digest(&candidate)?;
        let final_path = snapshots.join(format!("{source_sha256}.sqlite3"));
        let retained = final_path.try_exists()?;
        if retained {
            ensure!(
                digest(&final_path)? == source_sha256,
                "legacy snapshot digest changed"
            );
        }
        Ok((rows, source_sha256, final_path, retained))
    });
    let ((messages, state), source_sha256, final_path, retained) = match checked {
        Ok(checked) => checked,
        Err(error) => {
            return Err(discard_candidate(
                &snapshot_directory,
                candidate_name,
                file,
                error,
            ));
        }
    };
    if retained {
        snapshot_directory.remove_file(candidate_name, file)?;
    } else if let Err(error) = files::name(&final_path).and_then(|final_name| {
        snapshot_directory
            .publish_file(
                &snapshot_directory,
                candidate_name,
                &file,
                final_name,
                Publication::New,
            )
            .map_err(anyhow::Error::from)
    }) {
        return Err(discard_candidate(
            &snapshot_directory,
            candidate_name,
            file,
            error,
        ));
    }
    Ok(Some(LegacyImport {
        receipt: MigrationReceipt {
            source_sha256,
            snapshot: final_path,
            project_scope: selection.project_scope.into(),
            source_project_scope: selection.changed().then(|| selection.source_scope.into()),
            messages: messages.len(),
            state: state.len(),
        },
        messages,
        state,
    }))
}

/// Apply the selection's refusals to row names in one read view of the
/// pinned source. Returns whether any selected row exists.
fn precheck(source: &Connection, selection: Selection<'_>) -> Result<bool> {
    source.execute_batch("BEGIN")?;
    let mut selected = false;
    for (query, is_state) in [
        ("SELECT namespace FROM messages", false),
        ("SELECT key FROM state", true),
    ] {
        let mut statement = source.prepare(query)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            selected |= selection.select(&name, is_state)?.is_some();
        }
    }
    source.execute_batch("COMMIT")?;
    Ok(selected)
}

type SelectedRows = (Vec<LegacyMessage>, Vec<(String, String)>);

/// Copy one consistent source view into the candidate and select its rows;
/// this scan is authoritative because the source may change after a
/// pre-check. Every SQLite handle is closed before return, so a failed
/// candidate can be removed on every platform.
fn fill_candidate(
    source: Connection,
    pins: &SourcePins,
    candidate: &Path,
    file: &fs::File,
    selection: Selection<'_>,
) -> Result<SelectedRows> {
    let mut snapshot = Connection::open(candidate)?;
    // Backup sees accepted WAL content while preserving source DB/WAL data.
    let backup = Backup::new(&source, &mut snapshot)?;
    run_backup(|| backup.step(256), Instant::now)?;
    drop(backup);
    validate(&snapshot)?;
    let rows = select_rows(&snapshot, selection)?;
    drop(snapshot);
    drop(source);
    pins.verify()?;
    ensure!(
        !selection.explicit || !rows.0.is_empty() || !rows.1.is_empty(),
        LegacyImportRejected(LegacyImportRefusal::SourceChanged)
    );
    // Retain the writable candidate through SQLite close: Windows cannot flush
    // an unrelated read-only handle obtained after the backup.
    file.sync_all()?;
    Ok(rows)
}

fn select_rows(snapshot: &Connection, selection: Selection<'_>) -> Result<SelectedRows> {
    let project_scope = selection.project_scope;
    let mut messages = Vec::new();
    let mut statement = snapshot
        .prepare("SELECT sequence, namespace, role, content FROM messages ORDER BY sequence")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let namespace: String = row.get(1)?;
        let Some(suffix) = selection.select(&namespace, false)? else {
            continue;
        };
        let message = LegacyMessage {
            sequence: row.get(0)?,
            namespace: format!("{project_scope}/{suffix}"),
            role: row.get(2)?,
            content: row.get(3)?,
        };
        ensure!(
            message.sequence > 0,
            "legacy message sequence must be positive"
        );
        identifier("namespace", &message.namespace, 1024)?;
        identifier("role", &message.role, 128)?;
        messages.push(message);
    }
    drop(rows);
    drop(statement);
    let mut state = Vec::new();
    let mut statement =
        snapshot.prepare("SELECT key, value FROM state ORDER BY key COLLATE BINARY")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let Some(suffix) = selection.select(&key, true)? else {
            continue;
        };
        let value: String = row.get(1)?;
        let key = format!("{project_scope}/{suffix}");
        identifier("state key", &key, 1024)?;
        serde_json::from_str::<serde_json::Value>(&value)
            .context("legacy state contains invalid JSON")?;
        state.push((key, value));
    }
    Ok((messages, state))
}

/// Remove an unpublished candidate through its checked directory. The
/// original failure, including a typed refusal, stays the primary error.
fn discard_candidate(
    directory: &Directory,
    name: &std::ffi::OsStr,
    file: fs::File,
    error: anyhow::Error,
) -> anyhow::Error {
    match directory.remove_file(name, file) {
        Ok(()) => error,
        Err(cleanup) => error.context(format!(
            "legacy snapshot candidate cleanup also failed: {cleanup}"
        )),
    }
}

fn supported_moved_state_family(suffix: &str) -> bool {
    if matches!(suffix, "preferences" | "sessions") {
        return true;
    }
    if let Some(rest) = suffix.strip_prefix("session/") {
        let mut segments = rest.split('/');
        let Some(session) = segments.next() else {
            return false;
        };
        if session.is_empty() {
            return false;
        }
        return match (segments.next(), segments.next(), segments.next()) {
            (None, None, None) => true,
            (Some("last-local-submission"), None, None) => true,
            (Some("turn"), Some(turn), None) => !turn.is_empty(),
            _ => false,
        };
    }
    let mut segments = suffix.split('/');
    let (Some(mode), Some(kind), None) = (segments.next(), segments.next(), segments.next()) else {
        return false;
    };
    kuru_core::Mode::ALL
        .iter()
        .any(|known| known.to_string() == mode)
        && matches!(kind, "topology" | "dream-undo" | "last-dream")
}

fn digest(path: &Path) -> Result<String> {
    use std::io::Read;
    let (parent, mut file) = files::read(path, Privacy::OwnerOnly)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    parent.verify(files::name(path)?, &file)?;
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

struct SourcePins {
    // This ordinary read policy preserves old Unix SQLite file permissions;
    // the parent is independently private and Windows validates each DACL.
    parent: Directory,
    files: Vec<(String, fs::File)>,
    absent: Vec<String>,
}
impl SourcePins {
    fn new(path: &Path) -> Result<Self> {
        #[cfg(not(windows))]
        files::directory(path)?;
        #[cfg(windows)]
        files::directory(path).map_err(|error| data_directory_error(path, error))?;
        let parent = Directory::open(path, Privacy::Inherited, NameRetention::Pinned)?;
        let mut pins = Self {
            parent,
            files: Vec::new(),
            absent: Vec::new(),
        };
        for name in [
            "memory.sqlite3",
            "memory.sqlite3-wal",
            "memory.sqlite3-shm",
            "memory.sqlite3-journal",
        ] {
            match pins.parent.read(std::ffi::OsStr::new(name)) {
                Ok(file) => {
                    #[cfg(windows)]
                    kuru_platform::fs::require_private(&file)?;
                    pins.files.push((name.into(), file));
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && name != "memory.sqlite3" =>
                {
                    pins.absent.push(name.into())
                }
                Err(error) => return Err(error).context("pin original SQLite memory and sidecars"),
            }
        }
        pins.verify()?;
        Ok(pins)
    }

    fn verify(&self) -> Result<()> {
        for (name, file) in &self.files {
            self.parent
                .verify(std::ffi::OsStr::new(name), file)
                .context("legacy SQLite source or sidecar was replaced")?;
        }
        // A missing SHM can legitimately be created by SQLite. Validate its
        // resulting identity/type/privacy; do not call this a custom VFS.
        for name in &self.absent {
            match self.parent.read(std::ffi::OsStr::new(name)) {
                Ok(file) => {
                    #[cfg(windows)]
                    kuru_platform::fs::require_private(&file)?;
                    self.parent.verify(std::ffi::OsStr::new(name), &file)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("validate newly created SQLite sidecar"),
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn data_directory_error(path: &Path, error: Error) -> Error {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
    {
        return error.context(format!(
            "memory data directory {path:?} is not owner-private; correct this directory's owner-only access with Windows file security settings and retry"
        ));
    }
    error
}

fn validate(connection: &Connection) -> Result<()> {
    validate_identity(connection)?;
    let health: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        health == "ok",
        "legacy memory integrity check failed: {health}"
    );
    Ok(())
}

/// Kuru identity, supported version and both tables, without a full scan.
fn validate_identity(connection: &Connection) -> Result<()> {
    let identity: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        identity == 0x4b55_5255,
        "legacy database does not identify itself as Kuru memory"
    );
    ensure!(
        version == 1,
        "unsupported legacy memory schema version {version}"
    );
    connection.prepare("SELECT sequence, namespace, role, content FROM messages")?;
    connection.prepare("SELECT key, value FROM state")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryStore, test_support};
    use serde_json::json;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    /// A clock advanced by a fixed step on every read.
    fn stepping_clock(step: Duration) -> impl FnMut() -> Instant {
        let mut current = Instant::now();
        move || {
            current += step;
            current
        }
    }

    #[test]
    fn snapshot_that_keeps_progressing_completes_past_the_stall_bound() {
        let mut remaining = 100;
        let mut steps = 0;
        run_backup(
            || {
                steps += 1;
                if remaining == 0 {
                    return Ok(StepResult::Done);
                }
                remaining -= 1;
                Ok(StepResult::More)
            },
            stepping_clock(Duration::from_secs(10)),
        )
        .unwrap();
        assert_eq!(steps, 101);
    }

    #[test]
    fn snapshot_stalled_on_a_busy_source_fails_naming_the_stall() {
        let mut steps = 0;
        let error = run_backup(
            || {
                steps += 1;
                Ok(StepResult::Busy)
            },
            stepping_clock(Duration::from_secs(11)),
        )
        .unwrap_err();
        assert!(steps <= 3, "{steps}");
        assert_eq!(
            error.to_string(),
            "legacy memory snapshot made no progress for 30 s while the source was busy or locked; retry when it is free"
        );
    }

    #[test]
    fn snapshot_step_error_propagates() {
        let error = run_backup(
            || Err(rusqlite::Error::InvalidQuery),
            stepping_clock(Duration::from_secs(1)),
        )
        .unwrap_err();
        assert!(
            error.downcast_ref::<rusqlite::Error>().is_some(),
            "{error:#}"
        );
    }

    fn fixture() -> test_support::TempDir {
        test_support::TempDir::new("kuru-legacy memory café 東京-", None).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn existing_store_and_migration_failure_keeps_native_owner_privacy_guidance() {
        let error = data_directory_error(
            Path::new(r"C:\\kuru-memory-store"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into(),
        );
        let text = format!("{error:#}");
        assert!(text.contains("owner-only access"), "{text}");
        assert!(text.contains("Windows file security"), "{text}");
        assert!(!text.contains("mode 0700"), "{text}");
        assert!(!text.contains("chmod"), "{text}");
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied),
            "{text}"
        );
    }
    fn legacy(path: &Path) -> Connection {
        let database = Connection::open(path).unwrap();
        database.execute_batch("PRAGMA journal_mode=WAL; PRAGMA application_id=1263882837; PRAGMA user_version=1; CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL); CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);").unwrap();
        // The decimal value is set from the identifier too, avoiding fixture drift.
        database
            .pragma_update(None, "application_id", 0x4b55_5255_i64)
            .unwrap();
        database
    }

    #[test]
    fn inventory_reads_several_scopes_and_committed_wal_without_activation() {
        let root = fixture();
        let first = format!("project/{}", "a".repeat(64));
        let second = format!("project/{}", "b".repeat(64));
        let source = legacy(&root.path().join("memory.sqlite3"));
        source
            .execute(
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','accepted WAL')",
                [format!("{first}/transcript/one")],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO state (key,value) VALUES (?1,'{}')",
                [format!("{second}/preferences")],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO state (key,value) VALUES ('unattributed','{}')",
                [],
            )
            .unwrap();
        let originals: Vec<_> = ["memory.sqlite3", "memory.sqlite3-wal"]
            .into_iter()
            .map(|name| (name, fs::read(root.path().join(name)).unwrap()))
            .collect();
        let found = inventory(root.path()).unwrap().unwrap();
        assert_eq!(
            found,
            LegacyInventory {
                scopes: vec![
                    InventoryScope {
                        scope: first,
                        messages: 1,
                        state: 0,
                    },
                    InventoryScope {
                        scope: second,
                        messages: 0,
                        state: 1,
                    },
                ],
                unresolved_rows: 1,
            }
        );
        for (name, bytes) in originals {
            assert_eq!(fs::read(root.path().join(name)).unwrap(), bytes);
        }
        assert!(!root.path().join("memory").exists());
    }

    #[test]
    fn inventory_rejects_multibyte_name_above_byte_limit() {
        let root = fixture();
        let source = legacy(&root.path().join("memory.sqlite3"));
        let long_name = "é".repeat(513);
        assert_eq!(long_name.chars().count(), 513);
        assert!(long_name.len() > 1_024);
        source
            .execute(
                "INSERT INTO state (key,value) VALUES (?1,'{}')",
                [long_name],
            )
            .unwrap();
        let error = inventory(root.path()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<LegacyInventoryRefusal>(),
            Some(&LegacyInventoryRefusal::NameLimit),
            "{error:#}"
        );
        assert!(!root.path().join("memory").exists());
    }

    #[test]
    fn interrupted_inventory_returns_no_partial_scope_list_or_activation() {
        let root = fixture();
        let source = legacy(&root.path().join("memory.sqlite3"));
        let scope = format!("project/{}", "a".repeat(64));
        source
            .execute(
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','private row')",
                [format!("{scope}/transcript/one")],
            )
            .unwrap();
        let before_db = fs::read(root.path().join("memory.sqlite3")).unwrap();
        let before_wal = fs::read(root.path().join("memory.sqlite3-wal")).unwrap();
        let error = inventory_with_deadline(root.path(), Instant::now() - Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<LegacyInventoryRefusal>(),
            Some(&LegacyInventoryRefusal::Deadline),
            "{error:#}"
        );
        assert_eq!(
            fs::read(root.path().join("memory.sqlite3")).unwrap(),
            before_db
        );
        assert_eq!(
            fs::read(root.path().join("memory.sqlite3-wal")).unwrap(),
            before_wal
        );
        assert!(!root.path().join("memory").exists());
    }

    /// Names retained under `memory/legacy`, including any unpublished
    /// candidate; an absent directory has none.
    fn legacy_entries(root: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = match fs::read_dir(root.join("memory/legacy")) {
            Ok(entries) => entries.map(|entry| entry.unwrap().file_name()).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("read legacy snapshots: {error}"),
        };
        names.sort();
        names
    }

    fn refusal(error: &anyhow::Error) -> Option<LegacyImportRefusal> {
        error
            .downcast_ref::<LegacyImportRejected>()
            .map(|rejected| rejected.0)
    }

    #[test]
    fn explicit_scope_mapping_changes_only_checked_names_and_records_provenance() {
        let root = fixture();
        let source_scope = format!("project/{}", "c".repeat(64));
        let target_scope = format!("project/{}", "d".repeat(64));
        let source = legacy(&root.path().join("memory.sqlite3"));
        source
            .execute(
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user',?2)",
                rusqlite::params![
                    format!("{source_scope}/transcript/one"),
                    format!("leave {source_scope} in ordinary text")
                ],
            )
            .unwrap();
        let opaque = format!(
            r#"{{"id":"one","mode":"ifs","turns":0,"label":"{source_scope}/ordinary label"}}"#
        );
        source
            .execute(
                "INSERT INTO state (key,value) VALUES (?1,?2)",
                rusqlite::params![format!("{source_scope}/session/one"), opaque],
            )
            .unwrap();
        let imported = prepare_explicit(root.path(), &target_scope, &source_scope)
            .unwrap()
            .unwrap();
        assert_eq!(
            imported.messages[0].namespace,
            format!("{target_scope}/transcript/one")
        );
        assert_eq!(
            imported.messages[0].content,
            format!("leave {source_scope} in ordinary text")
        );
        assert_eq!(imported.state[0].0, format!("{target_scope}/session/one"));
        assert_eq!(imported.state[0].1, opaque);
        assert_eq!(imported.receipt.project_scope, target_scope);
        assert_eq!(
            imported.receipt.source_project_scope.as_deref(),
            Some(source_scope.as_str())
        );
        assert!(root.path().join("memory.sqlite3").exists());
        assert!(imported.receipt.snapshot.exists());
        let published = legacy_entries(root.path());
        assert_eq!(
            published,
            [imported.receipt.snapshot.file_name().unwrap().to_owned()]
        );

        source
            .execute(
                "INSERT INTO state (key,value) VALUES (?1,?2)",
                rusqlite::params![
                    format!("{source_scope}/other"),
                    format!(r#"{{"future_payload":{{"actor_id":"{source_scope}/actor/one"}}}}"#)
                ],
            )
            .unwrap();
        let error = prepare_explicit(root.path(), &target_scope, &source_scope).unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(LegacyImportRefusal::UnsupportedMovedState),
            "{error:#}"
        );
        // A refusal copies nothing: no candidate and no new snapshot.
        assert_eq!(legacy_entries(root.path()), published);
        let same_scope = prepare(root.path(), &source_scope).unwrap().unwrap();
        assert_eq!(same_scope.state.len(), 2);
        assert_eq!(same_scope.receipt.source_project_scope, None);
        let published = legacy_entries(root.path());
        assert_eq!(published.len(), 2);
        source
            .execute(
                "DELETE FROM state WHERE key=?1",
                [format!("{source_scope}/other")],
            )
            .unwrap();
        source.execute(
            "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','target already present')",
            [format!("{target_scope}/transcript/other")],
        ).unwrap();
        let error = prepare_explicit(root.path(), &target_scope, &source_scope).unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(LegacyImportRefusal::DestinationCollision),
            "{error:#}"
        );
        assert_eq!(legacy_entries(root.path()), published);

        // No selected row: refused before any snapshot is copied.
        let absent = format!("project/{}", "e".repeat(64));
        let unrelated = format!("project/{}", "f".repeat(64));
        let error = prepare_explicit(root.path(), &unrelated, &absent).unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(LegacyImportRefusal::SourceChanged),
            "{error:#}"
        );
        assert_eq!(legacy_entries(root.path()), published);
    }

    #[test]
    fn ambiguous_scope_refuses_only_explicit_import_and_retains_nothing() {
        let root = fixture();
        let scope = format!("project/{}", "c".repeat(64));
        let target = format!("project/{}", "d".repeat(64));
        let source = legacy(&root.path().join("memory.sqlite3"));
        for namespace in [format!("{scope}/transcript/one"), format!("{scope}X/other")] {
            source
                .execute(
                    "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','row')",
                    [namespace],
                )
                .unwrap();
        }
        let before_db = fs::read(root.path().join("memory.sqlite3")).unwrap();
        for (project, selected) in [(&scope, &scope), (&target, &scope)] {
            let error = prepare_explicit(root.path(), project, selected).unwrap_err();
            assert_eq!(
                refusal(&error),
                Some(LegacyImportRefusal::AmbiguousSourceScope),
                "{error:#}"
            );
            assert!(legacy_entries(root.path()).is_empty());
        }
        assert_eq!(
            fs::read(root.path().join("memory.sqlite3")).unwrap(),
            before_db
        );

        // Automatic first open keeps its established rule: the extended row
        // is outside the exact scope and is skipped, not refused.
        let ordinary = prepare(root.path(), &scope).unwrap().unwrap();
        assert_eq!(ordinary.messages.len(), 1);
        assert_eq!(
            ordinary.messages[0].namespace,
            format!("{scope}/transcript/one")
        );
        assert_eq!(legacy_entries(root.path()).len(), 1);
    }

    #[test]
    fn failed_snapshot_scan_removes_its_candidate() {
        let root = fixture();
        let scope = format!("project/{}", "c".repeat(64));
        let source = legacy(&root.path().join("memory.sqlite3"));
        source
            .execute(
                "INSERT INTO state (key,value) VALUES (?1,'not json')",
                [format!("{scope}/preferences")],
            )
            .unwrap();
        // Ordinary import has no pre-check, so this fails in the snapshot scan
        // after the candidate exists.
        let error = prepare(root.path(), &scope).unwrap_err();
        assert!(format!("{error:#}").contains("invalid JSON"), "{error:#}");
        assert!(root.path().join("memory/legacy").is_dir());
        assert!(legacy_entries(root.path()).is_empty());
    }

    #[test]
    fn scope_proof_is_an_exact_bounded_existence_check() {
        let root = fixture();
        let scope = format!("project/{}", "c".repeat(64));
        assert!(!source_scope_proved(root.path(), &scope).unwrap());
        let source = legacy(&root.path().join("memory.sqlite3"));
        let state_only = format!("project/{}", "a".repeat(64));
        for (table, name) in [
            ("messages", format!("{scope}/transcript/one")),
            ("messages", format!("{scope}X/other")),
            ("state", format!("{state_only}/preferences")),
        ] {
            let query = if table == "messages" {
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','row')"
            } else {
                "INSERT INTO state (key,value) VALUES (?1,'{}')"
            };
            source.execute(query, [name]).unwrap();
        }
        assert!(source_scope_proved(root.path(), &scope).unwrap());
        assert!(source_scope_proved(root.path(), &state_only).unwrap());
        // A bare scope, a prefix of a scope and an extended name prove nothing.
        let bare = format!("project/{}", "b".repeat(64));
        source
            .execute(
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','row')",
                [format!("{bare}/")],
            )
            .unwrap();
        assert!(!source_scope_proved(root.path(), &bare).unwrap());
        assert!(!source_scope_proved(root.path(), &format!("project/{}", "c".repeat(63))).unwrap());
        let extended_only = format!("project/{}", "f".repeat(64));
        source
            .execute(
                "INSERT INTO messages (namespace,role,content) VALUES (?1,'user','row')",
                [format!("{extended_only}X/other")],
            )
            .unwrap();
        assert!(!source_scope_proved(root.path(), &extended_only).unwrap());
        assert!(!root.path().join("memory").exists());
    }

    #[cfg(unix)]
    #[test]
    fn public_legacy_directory_refuses_with_a_safe_local_remedy() {
        use std::os::unix::fs::PermissionsExt;

        let root = fixture();
        let source = root.path().join("memory.sqlite3");
        let original = b"legacy source";
        files::write(&source, original).unwrap();
        let mode = fs::metadata(root.path()).unwrap().permissions().mode();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let result = SourcePins::new(root.path());
        let rejected_mode = fs::metadata(root.path()).unwrap().permissions().mode();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(mode)).unwrap();
        let error = match result {
            Ok(_) => panic!("public legacy data directory must be rejected"),
            Err(error) => error,
        };
        let text = format!("{error:#}");
        assert!(text.contains("is not owner-private"), "{text}");
        assert!(text.contains("mode 0700"), "{text}");
        assert!(text.contains(&format!("{:?}", root.path())), "{text}");
        assert_eq!(rejected_mode & 0o777, 0o755);
        assert_eq!(fs::read(source).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn public_owner_directory_without_legacy_gets_remedy_without_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let root = fixture();
        let sentinel = root.path().join("keep.txt");
        fs::write(&sentinel, b"leave owner data untouched").unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let before = fs::metadata(root.path()).unwrap().permissions().mode() & 0o777;
        let text = format!("{:#}", files::directory(root.path()).unwrap_err());

        assert!(text.contains("memory data directory"), "{text}");
        assert!(text.contains("mode 0700"), "{text}");
        assert!(text.contains(&format!("{:?}", root.path())), "{text}");
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            before
        );
        assert_eq!(fs::read(sentinel).unwrap(), b"leave owner data untouched");
    }

    #[cfg(unix)]
    #[test]
    fn linked_and_foreign_directories_do_not_receive_mode_guidance() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let link = root.path().with_extension("link");
        symlink(root.path(), &link).unwrap();
        let text = format!("{:#}", files::directory(&link).unwrap_err());
        assert!(!text.contains("mode 0700"), "{text}");

        let foreign = Path::new("/");
        if fs::symlink_metadata(foreign).unwrap().uid() != nix::unistd::geteuid().as_raw() {
            let text = format!("{:#}", files::directory(foreign).unwrap_err());
            assert!(!text.contains("mode 0700"), "{text}");
        }
    }

    #[test]
    fn committed_wal_without_shm_rebuilds_coordination_and_preserves_database_bytes() {
        let origin = fixture();
        let restored = fixture();
        let scope = format!("project/{}", "7".repeat(64));
        let source = legacy(&origin.path().join("memory.sqlite3"));
        source.execute("INSERT INTO messages (namespace,role,content) VALUES (?1,'user','accepted in WAL')",
            [format!("{scope}/transcript")]).unwrap();
        assert!(origin.path().join("memory.sqlite3-shm").is_file());
        // The committed writer is idle while this fixture copies DB and WAL.
        // Omitting SHM models a legitimate restored legacy store, not an
        // immutable SQLite URI or a custom VFS.
        let originals: Vec<_> = ["memory.sqlite3", "memory.sqlite3-wal"]
            .into_iter()
            .map(|name| (name, fs::read(origin.path().join(name)).unwrap()))
            .collect();
        for (name, bytes) in &originals {
            files::write(&restored.path().join(name), bytes).unwrap();
        }
        assert!(!restored.path().join("memory.sqlite3-shm").exists());
        let imported = prepare(restored.path(), &scope).unwrap().unwrap();
        assert_eq!(imported.messages.len(), 1);
        assert_eq!(imported.messages[0].content, "accepted in WAL");
        for (name, original) in originals {
            assert_eq!(fs::read(restored.path().join(name)).unwrap(), original);
        }
        let snapshot = Connection::open_with_flags(
            &imported.receipt.snapshot,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            snapshot
                .query_row::<String, _, _>("SELECT content FROM messages", [], |row| row.get(0))
                .unwrap(),
            "accepted in WAL"
        );
    }

    #[test]
    fn held_sqlite_sidecar_identity_rejects_replacement_and_unsafe_new_sidecars() {
        let root = fixture();
        files::write(
            &root.path().join("memory.sqlite3"),
            b"identity fixture, never opened as SQLite",
        )
        .unwrap();
        files::write(&root.path().join("memory.sqlite3-wal"), b"original WAL").unwrap();
        let pins = SourcePins::new(root.path()).unwrap();
        #[cfg(unix)]
        {
            fs::rename(
                root.path().join("memory.sqlite3-wal"),
                root.path().join("retained-wal"),
            )
            .unwrap();
            files::write(&root.path().join("memory.sqlite3-wal"), b"replacement WAL").unwrap();
            assert!(pins.verify().is_err());
        }
        #[cfg(windows)]
        {
            // Windows pins deny delete sharing. Replacing either the database
            // or WAL is rejected while these exact source handles remain open.
            for (name, bytes) in [
                (
                    "memory.sqlite3",
                    b"identity fixture, never opened as SQLite".as_slice(),
                ),
                ("memory.sqlite3-wal", b"original WAL".as_slice()),
            ] {
                let moved = root.path().join(format!("retained-{name}"));
                assert!(fs::rename(root.path().join(name), &moved).is_err());
                assert!(!moved.exists());
                assert_eq!(fs::read(root.path().join(name)).unwrap(), bytes);
                pins.verify().unwrap();
            }
        }
        drop(pins);
        #[cfg(windows)]
        {
            fs::rename(
                root.path().join("memory.sqlite3-wal"),
                root.path().join("retained-wal"),
            )
            .unwrap();
            files::write(&root.path().join("memory.sqlite3-wal"), b"replacement WAL").unwrap();
        }
        assert_eq!(
            fs::read(root.path().join("retained-wal")).unwrap(),
            b"original WAL"
        );
        let pins = SourcePins::new(root.path()).unwrap();
        fs::create_dir(root.path().join("memory.sqlite3-shm")).unwrap();
        assert!(pins.verify().is_err());
        assert_eq!(
            fs::read(root.path().join("memory.sqlite3-wal")).unwrap(),
            b"replacement WAL"
        );
    }

    #[tokio::test]
    async fn wal_import_preserves_original_other_projects_order_and_opaque_json() {
        let directory = fixture();
        let scope = format!("project/{}", "b".repeat(64));
        let other_scope = format!("project/{}", "c".repeat(64));
        let path = directory.path().join("memory.sqlite3");
        let source = legacy(&path);
        let namespace = format!("{scope}/identity/部品");
        for (sequence, content) in [(3, "first\0東京"), (9, "second")] {
            source
                .execute(
                    "INSERT INTO messages VALUES (?1,?2,'tool',?3)",
                    rusqlite::params![sequence, namespace, content],
                )
                .unwrap();
        }
        source
            .execute(
                "INSERT INTO messages VALUES (10,?1,'user','other project')",
                [format!("{other_scope}/transcript/one")],
            )
            .unwrap();
        let key = format!("{scope}/preferences");
        let raw_json = " { \"mode\" : \"jungian\", \"nested\" : [1,true,null] } ";
        source
            .execute(
                "INSERT INTO state VALUES (?1,?2)",
                rusqlite::params![key, raw_json],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO state VALUES (?1,'\"kept\"')",
                [format!("{other_scope}/preferences")],
            )
            .unwrap();
        assert!(directory.path().join("memory.sqlite3-wal").is_file());
        let original = fs::read(&path).unwrap();
        let original_wal = fs::read(directory.path().join("memory.sqlite3-wal")).unwrap();
        let prepared = prepare(directory.path(), &scope).unwrap().unwrap();
        assert_eq!(prepared.messages.len(), 2);
        assert_eq!(prepared.messages[0].sequence, 3);
        assert_eq!(prepared.messages[1].sequence, 9);
        assert_eq!(prepared.state, [(key.clone(), raw_json.into())]);
        let snapshot = Connection::open_with_flags(
            &prepared.receipt.snapshot,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            snapshot
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
                .unwrap(),
            3
        );
        assert_eq!(
            snapshot
                .query_row::<i64, _, _>("SELECT COUNT(*) FROM state", [], |row| row.get(0))
                .unwrap(),
            2
        );
        let retry = prepare(directory.path(), &scope).unwrap().unwrap();
        assert_eq!(retry.receipt, prepared.receipt);
        // A prepared legacy import selects cold creation, even with warmed caches.
        let options = test_support::warmed_open_options(directory.path().to_owned(), scope.clone())
            .await
            .unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        assert_eq!(
            store
                .history(&namespace, 10)
                .await
                .unwrap()
                .iter()
                .map(|message| message.plain_text().expect("legacy text"))
                .collect::<Vec<_>>(),
            ["first\0東京", "second"]
        );
        assert_eq!(
            store.get(&key).await.unwrap(),
            Some(json!({"mode":"jungian","nested":[1,true,null]}))
        );
        assert!(
            store
                .history(&format!("{other_scope}/transcript/one"), 10)
                .await
                .unwrap()
                .is_empty()
        );
        let revision = store.revision().await.unwrap();
        store.close().await.unwrap();

        let reopened = MemoryStore::open(options).await.unwrap();
        assert_eq!(reopened.revision().await.unwrap(), revision);
        assert_eq!(reopened.history(&namespace, 10).await.unwrap().len(), 2);
        reopened.close().await.unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            fs::read(directory.path().join("memory.sqlite3-wal")).unwrap(),
            original_wal
        );
    }

    #[test]
    fn corrupt_identity_schema_json_and_links_never_become_empty_memory() {
        let directory = fixture();
        let scope = format!("project/{}", "d".repeat(64));
        assert!(prepare(directory.path(), &scope).unwrap().is_none());
        let path = directory.path().join("memory.sqlite3");
        fs::write(&path, "not a database").unwrap();
        assert!(prepare(directory.path(), &scope).is_err());
        assert!(inventory(directory.path()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not a database");
        fs::remove_file(&path).unwrap();
        let database = legacy(&path);
        database.pragma_update(None, "application_id", 42).unwrap();
        assert!(
            prepare(directory.path(), &scope)
                .unwrap_err()
                .to_string()
                .contains("identify")
        );
        assert!(inventory(directory.path()).is_err());
        database
            .pragma_update(None, "application_id", 0x4b55_5255_i64)
            .unwrap();
        database.pragma_update(None, "user_version", 9).unwrap();
        assert!(
            prepare(directory.path(), &scope)
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
        assert!(inventory(directory.path()).is_err());
        database.pragma_update(None, "user_version", 1).unwrap();
        database
            .execute(
                "INSERT INTO state VALUES (?1,'broken-json')",
                [format!("{scope}/broken")],
            )
            .unwrap();
        assert!(
            prepare(directory.path(), &scope)
                .unwrap_err()
                .to_string()
                .contains("invalid JSON")
        );
        drop(database);
        let target = directory.path().join("actual.sqlite3");
        fs::rename(&path, &target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &path).unwrap();
        assert!(prepare(directory.path(), &scope).is_err());
        assert!(inventory(directory.path()).is_err());
        assert!(target.is_file());
    }

    #[tokio::test]
    async fn a_new_project_imports_empty_scope_without_losing_other_projects() {
        let directory = fixture();
        let scope = format!("project/{}", "e".repeat(64));
        let source = legacy(&directory.path().join("memory.sqlite3"));
        source.execute("INSERT INTO messages (namespace,role,content) VALUES ('project/other/transcript','user','preserved')", []).unwrap();
        // A prepared legacy import selects cold creation, even with warmed caches.
        let options = test_support::warmed_open_options(directory.path().to_owned(), scope)
            .await
            .unwrap();
        let store = MemoryStore::open(options).await.unwrap();
        assert!(
            store
                .history("project/other/transcript", 10)
                .await
                .unwrap()
                .is_empty()
        );
        store.append("new", "user", "first turn").await.unwrap();
        assert_eq!(
            store.history("new", 10).await.unwrap()[0].plain_text(),
            Some("first turn")
        );
        store.close().await.unwrap();
        assert_eq!(
            source
                .query_row::<String, _, _>("SELECT content FROM messages", [], |row| row.get(0))
                .unwrap(),
            "preserved"
        );
    }
}
