use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{MemoryConfig, Message};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Connection, MySqlConnection, MySqlPool, Row};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::{
    files,
    migration::{self, LegacyImport, MigrationReceipt},
    provision,
    server::{LifecycleLease, Server, ServerOptions},
};

#[cfg(test)]
#[path = "store/recovery_tests.rs"]
mod recovery_tests;

pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const AUTHOR: &str = "Kuru <memory@kuru.local>";

#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub data_dir: PathBuf,
    pub project_scope: String,
    pub config: MemoryConfig,
    pub read_only: bool,
    pub supervisor: Option<PathBuf>,
}
impl OpenOptions {
    pub fn new(data_dir: PathBuf, project_scope: String) -> Self {
        Self {
            data_dir,
            project_scope,
            config: MemoryConfig::default(),
            read_only: false,
            supervisor: None,
        }
    }
}

#[derive(Debug)]
struct Shared {
    server: Server,
    directory: PathBuf,
    project_scope: String,
    read_only: bool,
    write: Arc<Mutex<()>>,
    uncertain: StdMutex<Option<Pending>>,
    _permit: Option<OwnedSemaphorePermit>,
}

#[derive(Clone, Debug)]
struct Pending {
    pool: Arc<MySqlPool>,
    connection: u64,
    receipt: Receipt,
}

#[derive(Clone, Debug)]
enum Receipt {
    Operation(String),
    Promotion { base: String, target: String },
}

/// A cloneable view whose SQL connections always select the same Dolt branch.
/// Namespace access policy remains the caller's responsibility.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    shared: Arc<Shared>,
    pool: Arc<MySqlPool>,
    branch: String,
}
pub type MemoryView = MemoryStore;

#[derive(Debug)]
pub struct Candidate {
    live: MemoryStore,
    view: MemoryStore,
    base: String,
}
impl Candidate {
    pub fn view(&self) -> MemoryStore {
        self.view.clone()
    }
    pub fn base(&self) -> &str {
        &self.base
    }
    pub async fn promote(&self) -> Result<String> {
        self.live.writable()?;
        let guard = self.live.shared.write.clone().lock_owned().await;
        self.live.resolve_uncertain().await?;
        let live = self.live.clone();
        let view = self.view.clone();
        let base = self.base.clone();
        // Keep accepted promotion alive if the UI cancels while awaiting its reply.
        tokio::spawn(async move {
            let _guard = guard;
            let target = view.revision().await?;
            let current = live.revision().await?;
            if current == target {
                return Ok(current);
            }
            ensure!(
                current == base,
                "dream candidate is stale: live memory changed since its base"
            );
            let (mut connection, id) = owned_connection(&live.pool).await?;
            *live.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: live.pool.clone(),
                connection: id,
                receipt: Receipt::Promotion {
                    base,
                    target: target.clone(),
                },
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                    .bind(&view.branch)
                    .fetch_all(&mut connection),
            )
            .await;
            // Drop the actual socket, then wait for server-side session teardown.
            // An absent receipt is not a rollback while that session can commit.
            drop(connection);
            if live.resolve_uncertain().await? == Some(true) {
                return Ok(target);
            }
            result.context("Dolt promotion deadline exceeded")??;
            bail!("Dolt did not fast-forward to the candidate revision")
        })
        .await
        .context("memory promotion worker failed")?
    }
}

#[derive(Debug, Serialize)]
pub struct Revision {
    pub hash: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct MemoryStatus {
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub project: String,
    pub directory: PathBuf,
    pub branch: String,
    pub revision: String,
    pub read_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    format: u32,
    project_scope: String,
    initial_revision: String,
    migration: Option<MigrationReceipt>,
}

struct StoppedStage {
    // Hold the existing lifecycle inode through rename and directory fsync.
    _lease: LifecycleLease,
}

pub(crate) mod marker_fixture;

impl MemoryStore {
    pub fn exists(data_dir: &Path, project_scope: &str) -> Result<bool> {
        let path = project_directory(data_dir, project_scope)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "project memory must be a regular directory"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        read_activation(&path, project_scope)?;
        Ok(true)
    }

    pub async fn open(options: OpenOptions) -> Result<Self> {
        Self::open_inner(options, None, None, None).await
    }

    async fn open_inner(
        options: OpenOptions,
        temporary: Option<Arc<tempfile::TempDir>>,
        permit: Option<OwnedSemaphorePermit>,
        mut marker_pause: Option<marker_fixture::ReadyMarkerPause>,
    ) -> Result<Self> {
        ensure!(
            (1..=300).contains(&options.config.startup_timeout_secs),
            "memory startup timeout must be 1..=300 seconds"
        );
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        private_dir(&options.data_dir)?;
        let parent = directory.parent().context("project store has no parent")?;
        private_dir(parent)?;
        let locks = parent.join("locks");
        private_dir(&locks)?;
        let lock_directory = Directory::open(&locks, Privacy::OwnerOnly, NameRetention::Pinned)?;
        let name = directory.file_name().context("project store has no name")?;
        let lock = lock_directory.lock_file(name)?;
        let lock = acquire_lock(
            lock,
            Duration::from_secs(options.config.startup_timeout_secs),
        )
        .await?;
        lock_directory.verify(name, &lock)?;
        let binary =
            provision::provision(&options.config, &options.data_dir.join("tools/dolt")).await?;
        let supervisor = options
            .supervisor
            .clone()
            .unwrap_or(std::env::current_exe()?);
        let timeout = Duration::from_secs(options.config.startup_timeout_secs);
        #[cfg(unix)]
        let lifecycle_root = None;
        #[cfg(windows)]
        let lifecycle_root = Some(options.data_dir.join("memory/lifecycles"));
        let make_options = |path: PathBuf, read_only| ServerOptions {
            binary: binary.clone(),
            directory: path,
            project_scope: options.project_scope.clone(),
            supervisor: supervisor.clone(),
            timeout,
            read_only,
            retained: temporary.clone(),
            lifecycle_root: lifecycle_root.clone(),
        };
        if !Self::exists(&options.data_dir, &options.project_scope)? {
            ensure!(
                !options.read_only,
                "project memory has not been migrated or initialized; open Kuru normally first"
            );
            let data = options.data_dir.clone();
            let scope = options.project_scope.clone();
            let legacy =
                tokio::task::spawn_blocking(move || migration::prepare(&data, &scope)).await??;
            let recovered = recover_staging(
                &directory,
                &options.project_scope,
                legacy.as_ref(),
                &make_options,
            )
            .await?;
            let mut staging = if let Some(staging) = recovered {
                staging
            } else {
                let staging = parent.join(format!(
                    "{}.staging-{}",
                    name.to_string_lossy(),
                    Uuid::new_v4()
                ));
                private_dir(&staging)?;
                let server = Server::open(make_options(staging.clone(), false)).await?;
                let pool = server.pool("main").await?;
                let initialized = async {
                    initialize(&pool).await?;
                    if let Some(legacy) = &legacy {
                        import(&pool, legacy).await?;
                    }
                    let initial_revision = revision(&pool).await?;
                    let activation = Activation {
                        format: 1,
                        project_scope: options.project_scope.clone(),
                        initial_revision,
                        migration: legacy.as_ref().map(|legacy| legacy.receipt.clone()),
                    };
                    marker_fixture::reach(
                        &mut marker_pause,
                        marker_fixture::Boundary::Before,
                        &staging,
                        &activation,
                    )
                    .await?;
                    write_json(&staging.join("ready.json"), &activation)?;
                    marker_fixture::reach(
                        &mut marker_pause,
                        marker_fixture::Boundary::After,
                        &staging,
                        &activation,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                pool.close().await;
                let stopped = server.close().await;
                initialized?;
                stopped?;
                let lease =
                    Server::quiescence_at(&staging, lifecycle_root.as_deref(), timeout).await?;
                StoppedStage { _lease: lease }
            };
            // A live Dolt data directory must never be renamed.
            staging
                ._lease
                .move_to(&directory)
                .context("cannot activate validated Dolt memory")?;
            drop(staging);
        }
        read_activation(&directory, &options.project_scope)?;
        let server = Server::open(make_options(directory.clone(), options.read_only)).await?;
        let pool = server.pool("main").await?;
        validate_schema(&pool).await?;
        drop(lock);
        let shared = Arc::new(Shared {
            server,
            directory,
            project_scope: options.project_scope,
            read_only: options.read_only,
            write: Arc::new(Mutex::new(())),
            uncertain: StdMutex::new(None),
            _permit: permit,
        });
        Ok(Self {
            shared,
            pool,
            branch: "main".into(),
        })
    }

    /// Real isolated Dolt fixture. Missing runtime/helper is an error, never a skip.
    pub async fn temporary() -> Result<Self> {
        static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let permit = PERMITS
            .get_or_init(|| Arc::new(Semaphore::new(4)))
            .clone()
            .acquire_owned()
            .await?;
        let directory = tempfile::Builder::new().prefix("kuru-memory-").tempdir()?;
        let data = directory.path().join("private");
        let mut options = OpenOptions::new(data, format!("project/{}", "0".repeat(64)));
        options.config.cache_dir = Some(test_cache());
        options.config.offline = true;
        options.supervisor = Some(test_supervisor()?);
        Self::open_inner(options, Some(Arc::new(directory)), Some(permit), None).await
    }

    fn writable(&self) -> Result<()> {
        ensure!(!self.shared.read_only, "this memory view is read-only");
        ensure!(!self.pool.is_closed(), "memory store is closed");
        Ok(())
    }

    pub async fn append(&self, namespace: &str, role: &str, content: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        identifier("role", role, 128)?;
        self.mutate(
            "message",
            Mutation::Append {
                namespace: namespace.into(),
                role: role.into(),
                content: content.into(),
            },
        )
        .await
    }
    pub async fn history(&self, namespace: &str, limit: usize) -> Result<Vec<Message>> {
        identifier("namespace", namespace, 1024)?;
        let limit = i64::try_from(limit).context("history limit exceeds integer range")?;
        let rows = tokio::time::timeout(QUERY_TIMEOUT, sqlx::query(
            "SELECT role, content FROM (SELECT sequence, role, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        ).bind(namespace.as_bytes()).bind(limit).fetch_all(self.pool.as_ref())).await.context("memory read deadline exceeded")??;
        rows.into_iter()
            .map(|row| {
                Ok(Message {
                    role: String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?,
                    content: row.try_get("content")?,
                })
            })
            .collect()
    }
    pub async fn put(&self, key: &str, value: &Value) -> Result<()> {
        self.put_many(&[(key.into(), value.clone())]).await
    }
    pub async fn put_many(&self, values: &[(String, Value)]) -> Result<()> {
        let mut keys = BTreeSet::new();
        let mut encoded = Vec::with_capacity(values.len());
        for (key, value) in values {
            identifier("state key", key, 1024)?;
            ensure!(keys.insert(key), "duplicate state key in atomic update");
            encoded.push((key.clone(), serde_json::to_string(value)?));
        }
        if encoded.is_empty() {
            return Ok(());
        }
        self.mutate("state", Mutation::State(encoded)).await
    }
    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        identifier("state key", key, 1024)?;
        let value: Option<String> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(key.as_bytes())
                .fetch_optional(self.pool.as_ref()),
        )
        .await
        .context("memory read deadline exceeded")??;
        value
            .map(|value| serde_json::from_str(&value).context("stored state contains invalid JSON"))
            .transpose()
    }
    pub async fn clear(&self, namespace: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        self.mutate("clear conversation", Mutation::Clear(namespace.into()))
            .await
    }
    async fn mutate(&self, label: &str, mutation: Mutation) -> Result<()> {
        self.writable()?;
        let guard = self.shared.write.clone().lock_owned().await;
        self.resolve_uncertain().await?;
        let store = self.clone();
        let label = label.to_owned();
        tokio::spawn(async move {
            let _guard = guard;
            let operation = Uuid::new_v4().to_string();
            let (mut connection, id) = owned_connection(&store.pool).await?;
            *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: store.pool.clone(),
                connection: id,
                receipt: Receipt::Operation(operation.clone()),
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                apply(&mut connection, &operation, &label, mutation),
            )
            .await;
            drop(connection);
            if matches!(result, Ok(Ok(()))) {
                *store.shared.uncertain.lock().expect("uncertain lock") = None;
                return Ok(());
            }
            if store.resolve_uncertain().await? == Some(true) {
                return Ok(());
            }
            result.context("memory write deadline exceeded")??;
            bail!("memory mutation did not produce its durable receipt")
        })
        .await
        .context("memory write worker failed")?
    }
    async fn resolve_uncertain(&self) -> Result<Option<bool>> {
        let pending = self
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .clone();
        if let Some(pending) = pending {
            await_session_end(&pending.pool, pending.connection, QUERY_TIMEOUT)
                .await
                .context("memory outcome is uncertain; original SQL session has not finished")?;
            let committed = match pending.receipt {
                Receipt::Operation(operation) => {
                    operation_exists(&pending.pool, &operation).await?
                }
                Receipt::Promotion { base, target } => {
                    let observed = revision(&pending.pool).await?;
                    ensure!(
                        observed == target || observed == base,
                        "cannot reconcile promotion: live history diverged from both base and target"
                    );
                    observed == target
                }
            };
            *self.shared.uncertain.lock().expect("uncertain lock") = None;
            return Ok(Some(committed));
        }
        Ok(None)
    }
    pub async fn reconcile(&self) -> Result<()> {
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await.map(|_| ())
    }
    pub async fn begin_candidate(&self, label: &str) -> Result<Candidate> {
        self.writable()?;
        identifier("candidate label", label, 128)?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let base = self.revision().await?;
        let branch = format!("candidate_{}", Uuid::new_v4().simple());
        tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&branch)
                .bind(&base)
                .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("candidate creation deadline exceeded")??;
        let pool = self.shared.server.pool(&branch).await?;
        let view = Self {
            shared: self.shared.clone(),
            pool,
            branch,
        };
        Ok(Candidate {
            live: self.clone(),
            view,
            base,
        })
    }
    pub async fn revision(&self) -> Result<String> {
        revision(&self.pool).await
    }
    pub async fn revisions(&self, limit: usize) -> Result<Vec<Revision>> {
        let limit = i64::try_from(limit).context("revision limit exceeds integer range")?;
        let rows = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("SELECT commit_hash, message FROM dolt_log LIMIT ?")
                .bind(limit)
                .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("revision read deadline exceeded")??;
        rows.into_iter()
            .map(|row| {
                Ok(Revision {
                    hash: row.try_get("commit_hash")?,
                    message: row.try_get("message")?,
                })
            })
            .collect()
    }
    pub async fn status(&self) -> Result<MemoryStatus> {
        Ok(MemoryStatus {
            engine: "dolt",
            engine_version: provision::DOLT_VERSION,
            project: self.shared.project_scope.clone(),
            directory: self.shared.directory.clone(),
            branch: self.branch.clone(),
            revision: self.revision().await?,
            read_only: self.shared.read_only,
        })
    }
    pub async fn close(&self) -> Result<()> {
        let _guard = self.shared.write.lock().await;
        self.shared.server.close().await
    }
}

#[derive(Debug)]
enum Mutation {
    Append {
        namespace: String,
        role: String,
        content: String,
    },
    State(Vec<(String, String)>),
    Clear(String),
}

async fn apply(
    connection: &mut MySqlConnection,
    operation: &str,
    label: &str,
    mutation: Mutation,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    match mutation {
        Mutation::Append {
            namespace,
            role,
            content,
        } => {
            sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                .bind(namespace.as_bytes())
                .bind(role.as_bytes())
                .bind(content)
                .execute(&mut *transaction)
                .await?;
        }
        Mutation::State(values) => {
            for (key, value) in values {
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)").bind(key.as_bytes()).bind(value).execute(&mut *transaction).await?;
            }
        }
        Mutation::Clear(namespace) => {
            sqlx::query("DELETE FROM messages WHERE namespace = ?")
                .bind(namespace.as_bytes())
                .execute(&mut *transaction)
                .await?;
        }
    }
    sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
        .bind(operation)
        .bind(label)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!("{label} [{operation}]"))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}
async fn owned_connection(pool: &MySqlPool) -> Result<(MySqlConnection, u64)> {
    let mut connection = pool.acquire().await?.detach();
    let id = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut connection),
    )
    .await
    .context("memory connection identity deadline exceeded")??;
    Ok((connection, id))
}

async fn await_session_end(pool: &MySqlPool, id: u64, duration: Duration) -> Result<()> {
    tokio::time::timeout(duration, async {
        loop {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
            )
            .bind(id)
            .fetch_one(pool)
            .await?;
            if active == 0 {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("memory SQL session teardown deadline exceeded")?
}

async fn operation_exists(pool: &MySqlPool, operation: &str) -> Result<bool> {
    let result: Option<String> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT id FROM operations WHERE id = ?")
            .bind(operation)
            .fetch_optional(pool),
    )
    .await
    .context("memory reconciliation deadline exceeded")??;
    Ok(result.is_some())
}
async fn revision(pool: &MySqlPool) -> Result<String> {
    Ok(tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(pool),
    )
    .await
    .context("revision deadline exceeded")??)
}
async fn initialize(pool: &MySqlPool) -> Result<()> {
    // Initialization is only called in a new, unpublished staging directory.
    for statement in [
        "CREATE TABLE kuru_schema (id INT PRIMARY KEY, version INT NOT NULL)",
        "INSERT INTO kuru_schema VALUES (1, 1)",
        "CREATE TABLE messages (sequence BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, namespace VARBINARY(1024) NOT NULL, role VARBINARY(128) NOT NULL, content LONGTEXT CHARACTER SET utf8mb4 NOT NULL, INDEX messages_namespace_sequence (namespace, sequence))",
        "CREATE TABLE state (`key` VARBINARY(1024) PRIMARY KEY, value LONGTEXT CHARACTER SET utf8mb4 NOT NULL)",
        "CREATE TABLE operations (id VARCHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY, label VARCHAR(128) NOT NULL)",
    ] {
        sqlx::query(statement).execute(pool).await?;
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', 'Initialize Kuru memory schema 1', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(pool)
        .await?;
    validate_schema(pool).await
}
async fn validate_schema(pool: &MySqlPool) -> Result<()> {
    let version: i32 = sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
        .fetch_one(pool)
        .await
        .context("database is not an initialized Kuru memory store")?;
    ensure!(
        version == 1,
        "unsupported Dolt memory schema version {version}"
    );
    sqlx::query("SELECT sequence, namespace, role, content FROM messages LIMIT 0")
        .fetch_all(pool)
        .await?;
    sqlx::query("SELECT `key`, value FROM state LIMIT 0")
        .fetch_all(pool)
        .await?;
    sqlx::query("SELECT id, label FROM operations LIMIT 0")
        .fetch_all(pool)
        .await?;
    Ok(())
}
async fn import(pool: &MySqlPool, legacy: &LegacyImport) -> Result<()> {
    // The old file contains every project; this may be a new, empty scope.
    // Its snapshot receipt is still retained in the activation record.
    if legacy.messages.is_empty() && legacy.state.is_empty() {
        return Ok(());
    }
    let mut transaction = pool.begin().await?;
    for message in &legacy.messages {
        sqlx::query(
            "INSERT INTO messages (sequence, namespace, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(message.sequence)
        .bind(message.namespace.as_bytes())
        .bind(message.role.as_bytes())
        .bind(&message.content)
        .execute(&mut *transaction)
        .await?;
    }
    for (key, value) in &legacy.state {
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(key.as_bytes())
            .bind(value)
            .execute(&mut *transaction)
            .await?;
    }
    // Compare actual rows before making the import a durable revision.
    let rows =
        sqlx::query("SELECT sequence, namespace, role, content FROM messages ORDER BY sequence")
            .fetch_all(&mut *transaction)
            .await?;
    ensure!(
        rows.len() == legacy.messages.len(),
        "import message count differs"
    );
    for (row, original) in rows.iter().zip(&legacy.messages) {
        ensure!(
            row.try_get::<i64, _>("sequence")? == original.sequence
                && row.try_get::<Vec<u8>, _>("namespace")? == original.namespace.as_bytes()
                && row.try_get::<Vec<u8>, _>("role")? == original.role.as_bytes()
                && row.try_get::<String, _>("content")? == original.content,
            "imported message differs from preserved snapshot"
        );
    }
    let rows = sqlx::query("SELECT `key`, value FROM state ORDER BY `key`")
        .fetch_all(&mut *transaction)
        .await?;
    ensure!(
        rows.len() == legacy.state.len(),
        "import state count differs"
    );
    for (row, (key, value)) in rows.iter().zip(&legacy.state) {
        ensure!(
            row.try_get::<Vec<u8>, _>("key")? == key.as_bytes()
                && row.try_get::<String, _>("value")? == *value,
            "imported state differs from preserved snapshot"
        );
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!(
            "Import preserved SQLite {}",
            legacy.receipt.source_sha256
        ))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Reuse a fully committed import only when its preserved source still matches.
/// Partial or superseded stores are stopped and moved aside, never overwritten.
async fn recover_staging(
    directory: &Path,
    scope: &str,
    legacy: Option<&LegacyImport>,
    options: &impl Fn(PathBuf, bool) -> ServerOptions,
) -> Result<Option<StoppedStage>> {
    let parent = directory.parent().context("project store has no parent")?;
    let prefix = format!(
        "{}.staging-",
        directory
            .file_name()
            .context("project store has no name")?
            .to_string_lossy()
    );
    let mut stages = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        ensure!(
            Uuid::parse_str(suffix).is_ok() && entry.file_type()?.is_dir(),
            "unrecognized memory staging path: {}",
            entry.path().display()
        );
        stages.push(entry.path());
        ensure!(
            stages.len() <= 64,
            "too many interrupted memory imports; preserve and inspect {}",
            parent.display()
        );
    }
    stages.sort();
    let mut recovered = None;
    for stage in stages {
        let marker_exists = fs::symlink_metadata(stage.join("ready.json")).is_ok();
        let activation = marker_exists
            .then(|| read_activation(&stage, scope))
            .transpose()?;
        let identity_exists = fs::symlink_metadata(stage.join("identity.json")).is_ok();
        if identity_exists {
            // A completed stage needs only inspection. It may still have an
            // owner, so the quiescence lease below is required after closing.
            // An incomplete bootstrap needs exclusive server ownership.
            let server = Server::open(options(stage.clone(), activation.is_some())).await?;
            let checked = async {
                if let Some(activation) = &activation {
                    let pool = server.pool("main").await?;
                    validate_schema(&pool).await?;
                    ensure!(
                        revision(&pool).await? == activation.initial_revision,
                        "interrupted import revision differs from its activation record"
                    );
                    let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
                        .fetch_one(pool.as_ref())
                        .await?;
                    ensure!(
                        dirty == 0,
                        "interrupted import has uncommitted changes; preserve {} before recovery",
                        stage.display()
                    );
                    pool.close().await;
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let stopped = server.close().await;
            checked?;
            stopped?;
        } else {
            let mut recognized = true;
            for entry in fs::read_dir(&stage)? {
                let entry = entry?;
                recognized &= entry.file_name() == "lifecycle.lock" && entry.file_type()?.is_file();
            }
            ensure!(
                activation.is_none() && recognized,
                "unrecognized interrupted import without server identity: {}",
                stage.display()
            );
        }
        let lease_options = options(stage.clone(), false);
        let mut lease = Server::quiescence_at(
            &stage,
            lease_options.lifecycle_root.as_deref(),
            lease_options.timeout,
        )
        .await?;
        let matches_source =
            activation
                .as_ref()
                .is_some_and(|activation| match (&activation.migration, legacy) {
                    (None, None) => true,
                    (Some(receipt), Some(legacy)) => {
                        receipt.source_sha256 == legacy.receipt.source_sha256
                            && receipt.project_scope == legacy.receipt.project_scope
                            && receipt.messages == legacy.receipt.messages
                            && receipt.state == legacy.receipt.state
                    }
                    _ => false,
                });
        if matches_source && recovered.is_none() {
            recovered = Some(StoppedStage { _lease: lease });
        } else {
            let preserved = parent.join("interrupted");
            private_dir(&preserved)?;
            let destination =
                preserved.join(stage.file_name().context("staging path has no name")?);
            ensure!(
                fs::symlink_metadata(&destination).is_err(),
                "interrupted import preservation path already exists"
            );
            lease.move_to(&destination)?;
            eprintln!(
                "Preserved an interrupted memory import at {}",
                destination.display()
            );
        }
    }
    Ok(recovered)
}

pub(crate) fn identifier(label: &str, value: &str, maximum: usize) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} must not be empty");
    ensure!(value.len() <= maximum, "{label} exceeds {maximum} bytes");
    ensure!(!value.contains('\0'), "{label} must not contain NUL");
    Ok(())
}
fn project_directory(data: &Path, scope: &str) -> Result<PathBuf> {
    let hash = scope
        .strip_prefix("project/")
        .context("memory project scope must start with project/")?;
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "memory project scope must contain a lowercase SHA256 digest"
    );
    Ok(data.join("memory").join(hash))
}
fn read_activation(directory: &Path, scope: &str) -> Result<Activation> {
    let marker = directory.join("ready.json");
    let bytes = files::read_bytes(&marker, 16 * 1024).context(
        "project memory has no activation record; preserve the store and repair it before opening",
    )?;
    let activation: Activation = serde_json::from_slice(&bytes)?;
    ensure!(
        activation.format == 1 && activation.project_scope == scope,
        "project memory activation identity does not match"
    );
    Ok(activation)
}
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    files::private_dir(path)
}
pub(crate) fn private_file(path: &Path) -> Result<File> {
    let parent = files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    match parent.create_new(files::name(path)?) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(parent.read_write(files::name(path)?)?)
        }
        Err(error) => Err(error.into()),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    files::write(path, &serde_json::to_vec(value)?)
}
async fn acquire_lock(file: File, duration: Duration) -> Result<File> {
    let deadline = Instant::now() + duration;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(error) => {
                return Err(anyhow::anyhow!(error))
                    .context("memory initialization is locked by another process");
            }
        }
    }
}
pub(crate) fn test_cache() -> PathBuf {
    std::env::var_os("KURU_DOLT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("kuru-dolt-test-cache"))
}
pub(crate) fn test_supervisor() -> Result<PathBuf> {
    if let Some(snapshot) = crate::test_support::prepared_supervisor()? {
        return Ok(snapshot);
    }
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .context("test executable has no parent")?;
    let directory = if parent.file_name().is_some_and(|name| name == "deps") {
        parent
            .parent()
            .context("test executable has no build directory")?
    } else {
        parent
    };
    let helper = directory.join(if cfg!(windows) {
        "kuru-memory.exe"
    } else {
        "kuru-memory"
    });
    ensure!(
        helper.is_file(),
        "Dolt supervisor fixture is missing at {}; run mise run //packages/kuru-memory:build",
        helper.display()
    );
    Ok(helper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn failed_database_batch_rolls_back_and_committed_receipt_reconciles() {
        let store = MemoryStore::temporary().await.unwrap();
        store.put("one", &json!(1)).await.unwrap();
        let operation = Uuid::new_v4().to_string();
        let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
        apply(
            &mut connection,
            &operation,
            "receipt",
            Mutation::State(vec![("two".into(), "2".into())]),
        )
        .await
        .unwrap();
        let before = store.revision().await.unwrap();
        // Duplicate operation identity fails AFTER the row update, proving SQL rollback.
        assert!(
            apply(
                &mut connection,
                &operation,
                "duplicate",
                Mutation::State(vec![("one".into(), "10".into())])
            )
            .await
            .is_err()
        );
        drop(connection);
        await_session_end(&store.pool, id, QUERY_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(store.get("one").await.unwrap(), Some(json!(1)));
        assert_eq!(store.revision().await.unwrap(), before);
        assert!(operation_exists(&store.pool, &operation).await.unwrap());
        assert!(
            !operation_exists(&store.pool, &Uuid::new_v4().to_string())
                .await
                .unwrap()
        );
        // Simulate the caller losing the commit reply: the durable operation is
        // authoritative and reconciliation drains it without replaying its write.
        *store.shared.uncertain.lock().unwrap() = Some(Pending {
            pool: store.pool.clone(),
            connection: id,
            receipt: Receipt::Operation(operation),
        });
        store.reconcile().await.unwrap();
        assert!(store.shared.uncertain.lock().unwrap().is_none());
        assert_eq!(store.get("two").await.unwrap(), Some(json!(2)));
        assert_eq!(store.revision().await.unwrap(), before);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn durable_store_reopens_readonly_and_rejects_schema_drift() {
        let directory = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "a".repeat(64));
        let options =
            crate::test_support::open_options(directory.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        store.append("retained", "user", "hello").await.unwrap();
        store.put("choice", &json!("jungian")).await.unwrap();
        let revision = store.revision().await.unwrap();
        store.close().await.unwrap();
        drop(store);
        let mut readonly = options.clone();
        readonly.read_only = true;
        let reader = MemoryStore::open(readonly).await.unwrap();
        assert_eq!(reader.revision().await.unwrap(), revision);
        assert_eq!(
            reader.history("retained", 10).await.unwrap()[0].content,
            "hello"
        );
        assert_eq!(reader.get("choice").await.unwrap(), Some(json!("jungian")));
        assert!(reader.put("choice", &json!("ifs")).await.is_err());
        assert!(reader.begin_candidate("dream").await.is_err());
        assert!(reader.status().await.unwrap().read_only);
        reader.close().await.unwrap();
        drop(reader);
        let store = MemoryStore::open(options.clone()).await.unwrap();
        sqlx::query("UPDATE kuru_schema SET version = 99")
            .execute(store.pool.as_ref())
            .await
            .unwrap();
        assert!(
            validate_schema(&store.pool)
                .await
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
        store.close().await.unwrap();
        assert!(MemoryStore::open(options).await.is_err());
    }

    #[tokio::test]
    async fn private_paths_and_stable_lock_fail_closed() {
        let directory = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "a".repeat(64));
        assert!(!MemoryStore::exists(directory.path(), &scope).unwrap());
        for invalid in ["project/../../escape", "project/ABC", "bad"] {
            assert!(project_directory(directory.path(), invalid).is_err());
        }
        let file = directory.path().join("file");
        fs::write(&file, "retained").unwrap();
        assert!(private_dir(&file).is_err());
        assert!(private_file(directory.path()).is_err());
        let link = directory.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&file, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&file, &link).unwrap();
        assert!(private_file(&link).is_err());
        assert!(private_dir(&link).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "retained");
        let lock_path = directory.path().join("lease");
        let held = private_file(&lock_path).unwrap();
        held.lock().unwrap();
        assert!(
            acquire_lock(private_file(&lock_path).unwrap(), Duration::from_millis(20))
                .await
                .is_err()
        );
        drop(held);
        acquire_lock(private_file(&lock_path).unwrap(), Duration::from_millis(20))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn real_directory_move_completion_error_reconciles_identity_before_stage_recovery() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "7".repeat(64));
        let options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        let active = project_directory(data.path(), &scope).unwrap();
        store.close().await.unwrap();
        drop(store);
        let namespace = cfg!(windows).then(|| data.path().join("memory/lifecycles"));
        let mut lease =
            Server::quiescence_at(&active, namespace.as_deref(), Duration::from_secs(5))
                .await
                .unwrap();
        let identity = lease.directory.identity();
        let marker = fs::read(active.join("ready.json")).unwrap();
        let occupied = active.with_file_name("unrelated destination café 東京");
        private_dir(&occupied).unwrap();
        files::write(&occupied.join("evidence"), b"unrelated retained bytes").unwrap();
        assert!(files::move_directory(&lease.directory, &occupied).is_err());
        assert_eq!(files::directory(&active).unwrap().identity(), identity);
        assert_eq!(
            fs::read(occupied.join("evidence")).unwrap(),
            b"unrelated retained bytes"
        );
        let stage = active.with_file_name(format!("{}.staging-{}", "7".repeat(64), Uuid::new_v4()));
        let observed = std::cell::Cell::new(false);
        lease
            .move_to_observed(&stage, |moved| {
                assert_eq!(moved.identity(), identity);
                assert_eq!(fs::read(moved.path().join("ready.json"))?, marker);
                assert!(!active.exists());
                observed.set(true);
                Err(anyhow::anyhow!(
                    "fixture completion error after the actual directory move"
                ))
            })
            .unwrap();
        assert!(
            observed.get(),
            "the fixture must cross the actual native move boundary"
        );
        assert_eq!(lease.directory.identity(), identity);
        assert!(
            Server::quiescence_at(&stage, namespace.as_deref(), Duration::from_millis(20))
                .await
                .is_err()
        );
        drop(lease);
        let recovered = MemoryStore::open(options).await.unwrap();
        assert_eq!(recovered.revision().await.unwrap(), initial);
        assert_eq!(files::directory(&active).unwrap().identity(), identity);
        assert!(!stage.exists());
        assert_eq!(fs::read(active.join("ready.json")).unwrap(), marker);
        assert_eq!(
            fs::read(occupied.join("evidence")).unwrap(),
            b"unrelated retained bytes"
        );
        recovered.close().await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_activation_reuses_the_committed_stage_and_preserves_incomplete_work() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "f".repeat(64));
        let options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        let active = project_directory(data.path(), &scope).unwrap();
        store.close().await.unwrap();
        drop(store);
        // Model interruption at the actual durable boundary: the complete store
        // and activation receipt exist, but directory publication did not happen.
        let staging =
            active.with_file_name(format!("{}.staging-{}", "f".repeat(64), Uuid::new_v4()));
        fs::rename(&active, &staging).unwrap();
        let incomplete =
            active.with_file_name(format!("{}.staging-{}", "f".repeat(64), Uuid::new_v4()));
        private_dir(&incomplete).unwrap();
        // The supervisor creates its stable lock before writing store identity.
        #[cfg(unix)]
        private_file(&incomplete.join("lifecycle.lock")).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        assert_eq!(
            store.revision().await.unwrap(),
            initial,
            "reuse the completed import without a second schema revision"
        );
        assert!(!staging.exists());
        assert!(!incomplete.exists());
        assert!(
            data.path()
                .join("memory/interrupted")
                .join(incomplete.file_name().unwrap())
                .is_dir()
        );
        assert!(MemoryStore::exists(data.path(), &scope).unwrap());
        store.close().await.unwrap();
        drop(store);
        fs::remove_file(active.join("ready.json")).unwrap();
        assert!(
            MemoryStore::exists(data.path(), &scope).is_err(),
            "unknown active data must not appear empty"
        );
        assert!(MemoryStore::open(options).await.is_err());
    }

    #[tokio::test]
    async fn recovery_cannot_move_a_stage_while_an_attached_owner_is_still_running() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "8".repeat(64));
        let mut options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        store.close().await.unwrap();
        drop(store);
        let active = project_directory(data.path(), &scope).unwrap();
        let stage = active.with_file_name(format!("{}.staging-{}", "8".repeat(64), Uuid::new_v4()));
        fs::rename(&active, &stage).unwrap();
        let binary = provision::provision(&options.config, &data.path().join("tools/dolt"))
            .await
            .unwrap();
        let owner = Server::open(ServerOptions {
            binary,
            directory: stage.clone(),
            project_scope: scope,
            supervisor: options.supervisor.clone().unwrap(),
            timeout: Duration::from_secs(30),
            read_only: false,
            retained: None,
            lifecycle_root: if cfg!(windows) {
                Some(data.path().join("memory/lifecycles"))
            } else {
                None
            },
        })
        .await
        .unwrap();
        options.config.startup_timeout_secs = 1;
        let error = MemoryStore::open(options.clone()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("lifecycle remains active"),
            "{error:#}"
        );
        assert!(stage.is_dir());
        assert!(!active.exists());
        let pool = owner.pool("main").await.unwrap();
        assert_eq!(revision(&pool).await.unwrap(), initial);
        pool.close().await;
        owner.close().await.unwrap();
        options.config.startup_timeout_secs = 30;
        let recovered = MemoryStore::open(options).await.unwrap();
        assert_eq!(recovered.revision().await.unwrap(), initial);
        assert!(!stage.exists());
        recovered.close().await.unwrap();
    }

    #[tokio::test]
    async fn stopped_store_backup_restores_revisions_and_candidate_history_in_a_new_data_directory()
    {
        fn copy_tree(source: &Path, target: &Path) {
            private_dir(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let destination = target.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &destination);
                } else {
                    assert!(entry.file_type().unwrap().is_file());
                    fs::copy(entry.path(), destination).unwrap();
                }
            }
        }
        let source = crate::test_support::tempdir().unwrap();
        let restored = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "9".repeat(64));
        let options =
            crate::test_support::open_options(source.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options).await.unwrap();
        store
            .append("session", "user", "retained transcript")
            .await
            .unwrap();
        let revision = store.revision().await.unwrap();
        let candidate = store.begin_candidate("unpublished dream").await.unwrap();
        let view = candidate.view();
        view.append("candidate", "assistant", "private retained branch")
            .await
            .unwrap();
        let branch = view.branch.clone();
        store.close().await.unwrap();
        drop(view);
        drop(candidate);
        drop(store);
        copy_tree(
            &source.path().join("memory"),
            &restored.path().join("memory"),
        );
        let options = crate::test_support::open_options(restored.path().to_owned(), scope).unwrap();
        let store = MemoryStore::open(options).await.unwrap();
        assert_eq!(store.revision().await.unwrap(), revision);
        assert_eq!(
            store.history("session", 10).await.unwrap()[0].content,
            "retained transcript"
        );
        assert!(store.history("candidate", 10).await.unwrap().is_empty());
        let candidate_pool = store.shared.server.pool(&branch).await.unwrap();
        let content: String =
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"candidate".as_slice())
                .fetch_one(candidate_pool.as_ref())
                .await
                .unwrap();
        assert_eq!(content, "private retained branch");
        store.close().await.unwrap();
    }
}
