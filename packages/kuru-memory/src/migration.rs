//! One-way, read-only import. The original and a consistent full snapshot survive.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    // macOS's /var is a symlink. Resolve ancestors while still refusing a linked
    // database leaf with SQLite's NOFOLLOW flag.
    let source = source
        .parent()
        .context("legacy memory has no parent")?
        .canonicalize()?
        .join("memory.sqlite3");
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
    drop(file);
    let mut snapshot = Connection::open(&candidate)?;
    // SQLite's backup API sees committed WAL content without changing the source.
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
    fs::File::open(&candidate)?.sync_all()?;
    let source_sha256 = digest(&candidate)?;
    let final_path = snapshots.join(format!("{source_sha256}.sqlite3"));
    if final_path.try_exists()? {
        ensure!(
            fs::symlink_metadata(&final_path)?.is_file(),
            "legacy snapshot is not a regular file"
        );
        ensure!(
            digest(&final_path)? == source_sha256,
            "legacy snapshot digest changed"
        );
        fs::remove_file(&candidate)?;
    } else {
        fs::rename(&candidate, &final_path)?;
    }
    fs::File::open(&snapshots)?.sync_all()?;
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
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
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

    fn legacy(path: &Path) -> Connection {
        let database = Connection::open(path).unwrap();
        database.execute_batch("PRAGMA journal_mode=WAL; PRAGMA application_id=1263882837; PRAGMA user_version=1; CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL); CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);").unwrap();
        // The decimal value is set from the identifier too, avoiding fixture drift.
        database
            .pragma_update(None, "application_id", 0x4b55_5255_i64)
            .unwrap();
        database
    }

    #[tokio::test]
    async fn wal_import_preserves_original_other_projects_order_and_opaque_json() {
        let directory = tempfile::tempdir().unwrap();
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
        drop(store);
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
        let directory = tempfile::tempdir().unwrap();
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
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(prepare(directory.path(), &scope).is_err());
        assert!(target.is_file());
    }

    #[tokio::test]
    async fn a_new_project_imports_empty_scope_without_losing_other_projects() {
        let directory = tempfile::tempdir().unwrap();
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
