use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use crate::Message;

const APPLICATION_ID: i64 = 0x4b55_5255;
const SCHEMA_VERSION: i64 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Cloneable process-local handle backed by SQLite's transactional storage.
///
/// Namespaces are opaque to this layer. Callers must enforce access policy;
/// knowing a namespace is not itself authorization to read another part's memory.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    connection: Arc<Mutex<Connection>>,
}

impl MemoryStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create memory directory {}", parent.display()))?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot create memory database {}", path.display()));
            }
        }
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "memory database must not be a symbolic link"
        );
        ensure!(metadata.is_file(), "memory database must be a regular file");
        // SQLite treats the bare filename ":memory:" specially. Resolve the
        // created file first so open() always means durable filesystem storage.
        let resolved = path
            .canonicalize()
            .with_context(|| format!("cannot resolve memory database {}", path.display()))?;
        let connection = Connection::open(&resolved)
            .with_context(|| format!("cannot open memory database {}", path.display()))?;
        Self::initialize(connection)
            .with_context(|| format!("invalid memory database {}", path.display()))
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        // Identify and migrate before changing journal policy on an existing file.
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .context("cannot begin memory schema initialization")?;
            let version: i64 =
                transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            let identity: i64 =
                transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
            if version == 0 && identity == 0 {
                let tables: i64 = transaction.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )?;
                ensure!(
                    tables == 0,
                    "unversioned database already contains a schema; refusing to adopt it"
                );
                transaction.execute_batch(
                    "CREATE TABLE messages (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                        namespace TEXT NOT NULL,
                        role TEXT NOT NULL,
                        content TEXT NOT NULL
                    );
                    CREATE INDEX messages_namespace_sequence ON messages(namespace, sequence);
                    CREATE TABLE state (
                        key TEXT PRIMARY KEY NOT NULL,
                        value TEXT NOT NULL
                    );",
                )?;
                transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            } else {
                ensure!(
                    identity == APPLICATION_ID,
                    "database does not identify itself as a Kuru memory store"
                );
                ensure!(
                    version == SCHEMA_VERSION,
                    "unsupported memory schema version {version}; this build supports {SCHEMA_VERSION}"
                );
            }
            transaction
                .commit()
                .context("cannot commit memory schema initialization")?;
        }
        let health: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        ensure!(health == "ok", "memory database integrity check failed");
        // Fail at open, rather than during a user turn, if a table was damaged or altered.
        connection.prepare("SELECT sequence, namespace, role, content FROM messages LIMIT 0")?;
        connection.prepare("SELECT key, value FROM state LIMIT 0")?;
        enable_wal(&connection, BUSY_TIMEOUT)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn append(&self, namespace: &str, role: &str, content: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        identifier("role", role, 128)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO messages (namespace, role, content) VALUES (?1, ?2, ?3)",
            params![namespace, role, content],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Return the most recent `limit` entries in original chronological order.
    pub fn history(&self, namespace: &str, limit: usize) -> Result<Vec<Message>> {
        identifier("namespace", namespace, 1024)?;
        let limit = i64::try_from(limit).context("history limit exceeds SQLite's integer range")?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT role, content FROM (
                SELECT sequence, role, content FROM messages
                WHERE namespace = ?1 ORDER BY sequence DESC LIMIT ?2
            ) ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![namespace, limit], |row| {
            Ok(Message {
                role: row.get(0)?,
                content: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn put(&self, key: &str, value: &Value) -> Result<()> {
        identifier("state key", key, 1024)?;
        let encoded = serde_json::to_string(value)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, encoded],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Commit a related set of state values atomically. Duplicate keys are errors.
    /// Read-modify-write coordination between independent processes remains the
    /// caller's responsibility (for example, an exclusive project writer lease).
    pub fn put_many(&self, values: &[(String, Value)]) -> Result<()> {
        let mut keys = BTreeSet::new();
        let mut encoded = Vec::with_capacity(values.len());
        for (key, value) in values {
            identifier("state key", key, 1024)?;
            ensure!(keys.insert(key), "duplicate state key in atomic update");
            encoded.push((key, serde_json::to_string(value)?));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?;
            for (key, value) in encoded {
                statement.execute(params![key, value])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Value>> {
        identifier("state key", key, 1024)?;
        let connection = self.lock()?;
        let encoded: Option<String> = connection
            .query_row("SELECT value FROM state WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        encoded
            .map(|encoded| {
                serde_json::from_str(&encoded).context("stored state contains invalid JSON")
            })
            .transpose()
    }

    /// Clear only this conversation namespace, preserving other histories and state.
    pub fn clear(&self, namespace: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM messages WHERE namespace = ?1", [namespace])?;
        transaction.commit()?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("memory connection lock was poisoned by an earlier panic"))
    }
}

fn enable_wal(connection: &Connection, timeout: Duration) -> Result<()> {
    // SQLite can skip its busy handler during journal-mode lock promotion to
    // avoid deadlock. Retry this single autocommit statement after it has been
    // finalized, so another initializer can finish its schema transaction.
    // Own the wait budget here rather than nesting a busy timeout per attempt.
    // https://www.sqlite.org/c3ref/busy_handler.html
    connection.busy_timeout(Duration::ZERO)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match connection
            .query_row::<String, _, _>("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        {
            Ok(journal) => {
                break if matches!(journal.as_str(), "wal" | "memory") {
                    Ok(())
                } else {
                    Err(anyhow!(
                        "SQLite could not enable durable journaling: journal mode is {journal}"
                    ))
                };
            }
            Err(error) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break Err(error).context("timed out enabling SQLite WAL journaling");
                }
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(error) => break Err(error).context("cannot enable SQLite WAL journaling"),
        }
    };
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .context("cannot restore SQLite busy timeout after WAL initialization")?;
    result
}

fn identifier(label: &str, value: &str, max_bytes: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max_bytes && !value.contains('\0'),
        "{label} must be nonempty, at most {max_bytes} bytes, and contain no NUL characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_transition_waits_for_reserved_lock_and_preserves_data() {
        use std::sync::mpsc::{self, RecvTimeoutError};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("contended.sqlite3");
        let holder = Connection::open(&path).unwrap();
        holder
            .execute_batch("CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES ('keep'); BEGIN IMMEDIATE;")
            .unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.busy_timeout(BUSY_TIMEOUT).unwrap();
        let busy = connection
            .query_row::<String, _, _>("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .unwrap_err();
        assert_eq!(
            busy.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );

        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = enable_wal(&connection, Duration::from_secs(2));
            finished_tx.send(result).unwrap();
            connection
        });
        started_rx.recv().unwrap();
        let early = finished_rx.recv_timeout(Duration::from_millis(50));
        holder.execute_batch("COMMIT").unwrap();
        let returned_early = early.is_ok();
        let result = match early {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => finished_rx.recv().unwrap(),
            Err(error) => panic!("transition worker disconnected: {error}"),
        };
        let connection = worker.join().unwrap();
        assert!(
            !returned_early,
            "WAL transition returned while a real competing write lock was held: {result:?}"
        );
        result.unwrap();
        assert_eq!(
            connection
                .query_row::<String, _, _>("SELECT value FROM retained", [], |row| row.get(0))
                .unwrap(),
            "keep"
        );
        assert_eq!(
            connection
                .pragma_query_value::<String, _>(None, "journal_mode", |row| row.get(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            connection
                .pragma_query_value::<i64, _>(None, "busy_timeout", |row| row.get(0))
                .unwrap(),
            5000
        );
    }

    #[test]
    fn wal_contention_expires_and_restores_normal_busy_handling() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("locked.sqlite3");
        let holder = Connection::open(&path).unwrap();
        holder
            .execute_batch("CREATE TABLE retained(value TEXT); BEGIN IMMEDIATE;")
            .unwrap();
        let connection = Connection::open(&path).unwrap();
        let timeout = Duration::from_millis(20);
        let start = Instant::now();
        let error = enable_wal(&connection, timeout).unwrap_err();
        assert!(start.elapsed() >= timeout);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(error.to_string().contains("timed out enabling SQLite WAL"));
        assert_eq!(
            error
                .downcast_ref::<rusqlite::Error>()
                .unwrap()
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        assert_eq!(
            connection
                .pragma_query_value::<i64, _>(None, "busy_timeout", |row| row.get(0))
                .unwrap(),
            5000
        );
        holder.execute_batch("ROLLBACK").unwrap();
        enable_wal(&connection, timeout).unwrap();
    }

    #[test]
    fn wal_nonbusy_errors_are_reported_without_waiting_or_mutating_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corrupt.sqlite3");
        let bytes = b"not a sqlite database";
        fs::write(&path, bytes).unwrap();
        let connection = Connection::open(&path).unwrap();
        let start = Instant::now();
        let error = enable_wal(&connection, Duration::from_secs(5)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(error.to_string().contains("cannot enable SQLite WAL"));
        assert_eq!(
            error
                .downcast_ref::<rusqlite::Error>()
                .unwrap()
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::NotADatabase)
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            connection
                .pragma_query_value::<i64, _>(None, "busy_timeout", |row| row.get(0))
                .unwrap(),
            5000
        );
    }

    #[test]
    fn poisoned_connection_fails_without_exposing_contents() {
        let store = MemoryStore::in_memory().unwrap();
        let other = store.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = other.connection.lock().unwrap();
                panic!("simulate a interrupted holder");
            })
            .join()
            .is_err()
        );
        let error = store.history("private", 1).unwrap_err().to_string();
        assert!(error.contains("poisoned"));
        assert!(store.put("state", &Value::Null).is_err());
    }
}
