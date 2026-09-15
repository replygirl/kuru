//! One-way, read-only import. The original and a consistent full snapshot survive.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Error, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crate::files;
use crate::store::{identifier, private_dir, private_file};

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
    pub messages: usize,
    pub state: usize,
}

pub(crate) fn prepare(data_dir: &Path, project_scope: &str) -> Result<Option<LegacyImport>> {
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
    let snapshots = data_dir.join("memory/legacy");
    private_dir(&snapshots)?;
    let candidate = snapshots.join(format!("{}.sqlite3", uuid::Uuid::new_v4()));
    let file = private_file(&candidate)?;
    let mut snapshot = Connection::open(&candidate)?;
    // Backup sees accepted WAL content while preserving source DB/WAL data.
    let backup = Backup::new(&source, &mut snapshot)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        ensure!(
            Instant::now() < deadline,
            "legacy memory snapshot deadline exceeded; close older writers and retry"
        );
        match backup.step(256)? {
            StepResult::Done => break,
            StepResult::More => {}
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    drop(backup);
    validate(&snapshot)?;
    let prefix = format!("{project_scope}/");
    let mut messages = Vec::new();
    let mut statement = snapshot
        .prepare("SELECT sequence, namespace, role, content FROM messages ORDER BY sequence")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let namespace: String = row.get(1)?;
        if !namespace.starts_with(&prefix) {
            continue;
        }
        let message = LegacyMessage {
            sequence: row.get(0)?,
            namespace,
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
        if !key.starts_with(&prefix) {
            continue;
        }
        let value: String = row.get(1)?;
        identifier("state key", &key, 1024)?;
        serde_json::from_str::<serde_json::Value>(&value)
            .context("legacy state contains invalid JSON")?;
        state.push((key, value));
    }
    drop(rows);
    drop(statement);
    drop(snapshot);
    drop(source);
    pins.verify()?;
    // Retain the writable candidate through SQLite close: Windows cannot flush
    // an unrelated read-only handle obtained after the backup.
    file.sync_all()?;
    let snapshot_directory = files::directory(&snapshots)?;
    snapshot_directory.verify(files::name(&candidate)?, &file)?;
    let source_sha256 = digest(&candidate)?;
    let final_path = snapshots.join(format!("{source_sha256}.sqlite3"));
    if final_path.try_exists()? {
        ensure!(
            digest(&final_path)? == source_sha256,
            "legacy snapshot digest changed"
        );
        snapshot_directory.remove_file(files::name(&candidate)?, file)?;
    } else {
        snapshot_directory.publish_file(
            &snapshot_directory,
            files::name(&candidate)?,
            &file,
            files::name(&final_path)?,
            Publication::New,
        )?;
    }
    Ok(Some(LegacyImport {
        receipt: MigrationReceipt {
            source_sha256,
            snapshot: final_path,
            project_scope: project_scope.into(),
            messages: messages.len(),
            state: state.len(),
        },
        messages,
        state,
    }))
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

pub(crate) fn data_directory_error(path: &Path, error: Error) -> Error {
    #[cfg(unix)]
    if let Ok(directory) = fs::symlink_metadata(path)
        && error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        && directory.is_dir()
        && directory.uid() == nix::unistd::geteuid().as_raw()
        && directory.permissions().mode() & 0o077 != 0
    {
        return error.context(format!(
            "memory data directory {path:?} is not owner-private; restrict this exact directory to mode 0700 (for example with chmod, using shell quoting) and retry"
        ));
    }
    #[cfg(windows)]
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
    let health: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        health == "ok",
        "legacy memory integrity check failed: {health}"
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

    fn fixture() -> test_support::TempDir {
        test_support::TempDir::new("kuru-legacy memory café 東京-", None).unwrap()
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
        let error = files::directory(root.path()).unwrap_err();
        let text = format!("{:#}", data_directory_error(root.path(), error));

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
        let error = files::directory(&link).unwrap_err();
        let text = format!("{:#}", data_directory_error(&link, error));
        assert!(!text.contains("mode 0700"), "{text}");

        let foreign = Path::new("/");
        if fs::symlink_metadata(foreign).unwrap().uid() != nix::unistd::geteuid().as_raw() {
            let error = files::directory(foreign).unwrap_err();
            let text = format!("{:#}", data_directory_error(foreign, error));
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
        let options =
            test_support::open_options(directory.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        assert_eq!(
            store
                .history(&namespace, 10)
                .await
                .unwrap()
                .iter()
                .map(|message| message.content.as_str())
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
        assert!(target.is_file());
    }

    #[tokio::test]
    async fn a_new_project_imports_empty_scope_without_losing_other_projects() {
        let directory = fixture();
        let scope = format!("project/{}", "e".repeat(64));
        let source = legacy(&directory.path().join("memory.sqlite3"));
        source.execute("INSERT INTO messages (namespace,role,content) VALUES ('project/other/transcript','user','preserved')", []).unwrap();
        let options = test_support::open_options(directory.path().to_owned(), scope).unwrap();
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
            store.history("new", 10).await.unwrap()[0].content,
            "first turn"
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
