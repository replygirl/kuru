//! Private, immutable Dolt schema migration registry.
//!
//! A migration changes a versioned database, never an activation record or the
//! sidecar identity protocol.  Keeping the definitions here makes the receipt
//! validator the authority for both cold discovery and ordinary startup.
use super::{AUTHOR, QUERY_TIMEOUT, revision, validate_schema_v1};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use sqlx::{MySqlPool, Row};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::server::Server;

pub(super) const CURRENT_VERSION: i32 = 6;
pub(super) const USAGE_CURRENT_VERSION: i32 = 4;
const RESERVED_PREFIX: &str = "kuru_migration_";
const USAGE_RESERVED_PREFIX: &str = "kuru_usage_migration_";
const INVENTORY_LIMIT: usize = 64;
const DEFINITION_LIMIT: usize = 64;
const FIELD_LIMIT: usize = 1024;
const MIGRATION_ID_LIMIT: usize = 128;
const RECEIPT_PROTOCOL: &str = "kuru.memory.migration.receipt.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MigrationBoundary {
    BeforeBranch,
    BeforeDdl,
    AfterDdl,
    BeforeCommit,
    BeforePublish,
}

#[cfg(test)]
#[derive(Clone)]
struct MigrationPause {
    boundary: MigrationBoundary,
    // Fixtures select one accepted migration step even when an open advances
    // through multiple ordered schema versions.
    reached_once: std::sync::Arc<std::sync::atomic::AtomicBool>,
    reached: std::sync::Arc<tokio::sync::Semaphore>,
    resume: std::sync::Arc<tokio::sync::Semaphore>,
    route_ready: std::sync::Arc<tokio::sync::Semaphore>,
    route_resume: std::sync::Arc<tokio::sync::Semaphore>,
    route_source: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<MySqlPool>>>>,
    metadata: std::sync::Arc<std::sync::Mutex<MigrationMetadata>>,
}

#[cfg(test)]
#[derive(Default)]
struct MigrationMetadata {
    branch: Option<String>,
    target: Option<String>,
}

#[derive(Clone)]
pub(super) struct MigrationRunnerHooks {
    #[cfg(test)]
    pause: Option<MigrationPause>,
    #[cfg(test)]
    route: Option<(MigrationBoundary, u16)>,
}

#[cfg(test)]
pub(super) struct MigrationPauseControl {
    reached: std::sync::Arc<tokio::sync::Semaphore>,
    resume: std::sync::Arc<tokio::sync::Semaphore>,
    route_ready: std::sync::Arc<tokio::sync::Semaphore>,
    route_resume: std::sync::Arc<tokio::sync::Semaphore>,
    route_source: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<MySqlPool>>>>,
    metadata: std::sync::Arc<std::sync::Mutex<MigrationMetadata>>,
}

impl std::fmt::Debug for MigrationRunnerHooks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("MigrationRunnerHooks");
        #[cfg(test)]
        {
            debug.field("pause", &self.pause.as_ref().map(|pause| pause.boundary));
            debug.field("route", &self.route);
        }
        debug.finish()
    }
}

impl MigrationRunnerHooks {
    fn none() -> Self {
        Self {
            #[cfg(test)]
            pause: None,
            #[cfg(test)]
            route: None,
        }
    }

    #[cfg(test)]
    pub(super) fn paused(boundary: MigrationBoundary) -> (Self, MigrationPauseControl) {
        let reached = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let resume = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let route_ready = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let route_resume = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let route_source = std::sync::Arc::new(std::sync::Mutex::new(None));
        let metadata = std::sync::Arc::new(std::sync::Mutex::new(MigrationMetadata::default()));
        (
            Self {
                pause: Some(MigrationPause {
                    boundary,
                    reached_once: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    reached: reached.clone(),
                    resume: resume.clone(),
                    route_ready: route_ready.clone(),
                    route_resume: route_resume.clone(),
                    route_source: route_source.clone(),
                    metadata: metadata.clone(),
                }),
                route: None,
            },
            MigrationPauseControl {
                reached,
                resume,
                route_ready,
                route_resume,
                route_source,
                metadata,
            },
        )
    }

    #[cfg(test)]
    pub(super) fn with_route(mut self, boundary: MigrationBoundary, proxy_port: u16) -> Self {
        self.route = Some((boundary, proxy_port));
        self
    }

    async fn reach(&self, boundary: MigrationBoundary) -> Result<()> {
        #[cfg(not(test))]
        {
            let _ = boundary;
            Ok(())
        }
        #[cfg(test)]
        let Some(pause) = self
            .pause
            .as_ref()
            .filter(|pause| pause.boundary == boundary)
        else {
            return Ok(());
        };
        #[cfg(test)]
        {
            if pause
                .reached_once
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(());
            }
            pause.reached.add_permits(1);
            pause
                .resume
                .acquire()
                .await
                .context("migration fixture resume channel closed")?
                .forget();
            Ok(())
        }
    }

    fn describe(&self, boundary: MigrationBoundary, branch: &str, target: Option<&str>) {
        #[cfg(not(test))]
        let _ = (boundary, branch, target);
        #[cfg(test)]
        if let Some(pause) = self.pause.as_ref().filter(|pause| {
            pause.boundary == boundary
                && !pause.reached_once.load(std::sync::atomic::Ordering::SeqCst)
        }) {
            let mut metadata = pause
                .metadata
                .lock()
                .expect("migration fixture metadata lock");
            metadata.branch = Some(branch.to_owned());
            metadata.target = target.map(str::to_owned);
        }
    }
}

#[cfg(test)]
impl MigrationPauseControl {
    pub(super) async fn route_source(&self) -> Result<std::sync::Arc<MySqlPool>> {
        self.route_ready
            .acquire()
            .await
            .context("migration fixture route channel closed")?
            .forget();
        self.route_source
            .lock()
            .expect("migration fixture route source lock")
            .clone()
            .context("migration fixture route observer missing")
    }

    pub(super) fn resume_route(&self) {
        self.route_resume.add_permits(1);
    }

    pub(super) async fn reached(&self) -> Result<()> {
        self.reached
            .acquire()
            .await
            .context("migration fixture reached channel closed")?
            .forget();
        Ok(())
    }

    pub(super) fn resume(&self) {
        self.resume.add_permits(1);
    }

    pub(super) fn branch_name(&self) -> Result<String> {
        self.metadata
            .lock()
            .expect("migration fixture metadata lock")
            .branch
            .clone()
            .context("migration fixture branch name missing")
    }

    pub(super) fn publication_target(&self) -> Result<String> {
        self.metadata
            .lock()
            .expect("migration fixture metadata lock")
            .target
            .clone()
            .context("migration fixture publication target missing")
    }
}

async fn routed_pool(
    direct: &MySqlPool,
    hooks: &MigrationRunnerHooks,
    boundary: MigrationBoundary,
) -> Result<Option<MySqlPool>> {
    #[cfg(not(test))]
    {
        let _ = (direct, hooks, boundary);
        Ok(None)
    }
    #[cfg(test)]
    {
        let Some((_, port)) = hooks.route.filter(|(selected, _)| *selected == boundary) else {
            return Ok(None);
        };
        if hooks.pause.as_ref().is_some_and(|pause| {
            pause.boundary == boundary
                && pause.reached_once.load(std::sync::atomic::Ordering::SeqCst)
        }) {
            return Ok(None);
        }
        ensure!(port != 0, "migration fixture proxy port is invalid");
        let options = direct
            .connect_options()
            .as_ref()
            .clone()
            .host("127.0.0.1")
            .port(port);
        if let Some(pause) = hooks
            .pause
            .as_ref()
            .filter(|pause| pause.boundary == boundary)
        {
            *pause
                .route_source
                .lock()
                .expect("migration fixture route source lock") =
                Some(std::sync::Arc::new(direct.clone()));
            pause.route_ready.add_permits(1);
            pause
                .route_resume
                .acquire()
                .await
                .context("migration fixture route resume channel closed")?
                .forget();
        }
        Ok(Some(
            sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(QUERY_TIMEOUT)
                .connect_with(options)
                .await
                .context("connect migration fixture proxy")?,
        ))
    }
}

async fn bounded_query<T>(
    query: impl std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
) -> Result<T> {
    tokio::time::timeout(QUERY_TIMEOUT, query)
        .await
        .context("Dolt migration query deadline exceeded")?
        .map_err(Into::into)
}

#[derive(Clone, Copy)]
struct Definition {
    from: i32,
    to: i32,
    id: &'static str,
    sql: &'static [&'static str],
    transform: &'static str,
    postcondition: &'static str,
    failed_status: &'static [StatusRow],
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StatusRow {
    table: &'static str,
    staged: i64,
    status: &'static str,
}

const V2: Definition = Definition {
    from: 1,
    to: 2,
    id: "kuru.memory.receipts.v2",
    sql: &[
        "CREATE TABLE kuru_migrations (version INT PRIMARY KEY, id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, digest CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, operation CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE)",
    ],
    transform: "none",
    postcondition: "version=2;exact receipt ID/digest/canonical UUID;v1 tables remain readable",
    failed_status: &[StatusRow {
        table: "kuru_migrations",
        staged: 0,
        status: "new table",
    }],
};

const V3: Definition = Definition {
    from: 2,
    to: 3,
    id: "kuru.memory.typed-messages.v3",
    sql: &[
        "ALTER TABLE messages ADD COLUMN content_format VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL DEFAULT 'text-v1'",
    ],
    transform: "existing message content is exact text-v1",
    postcondition: "version=3;messages content_format is required with text-v1 default;old content retained",
    failed_status: &[StatusRow {
        table: "messages",
        staged: 0,
        status: "modified",
    }],
};

const V4: Definition = Definition {
    from: 3,
    to: 4,
    id: "kuru.memory.retained-operation-receipts.v4",
    sql: &[
        "ALTER TABLE operations ADD COLUMN receipt_format TINYINT NOT NULL DEFAULT 0",
        "ALTER TABLE operations ADD COLUMN method VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL",
        "ALTER TABLE operations ADD COLUMN request_digest CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL",
        "ALTER TABLE operations ADD COLUMN result_ref VARCHAR(256) CHARACTER SET ascii COLLATE ascii_bin NULL",
    ],
    transform: "existing operation rows remain legacy receipts without fabricated request identity",
    postcondition: "version=4;indexed operation receipts retain legacy rows and compact managed request evidence",
    failed_status: &[StatusRow {
        table: "operations",
        staged: 0,
        status: "modified",
    }],
};

const V5: Definition = Definition {
    from: 4,
    to: 5,
    id: "kuru.memory.session-provenance.v5",
    sql: &[
        "ALTER TABLE messages ADD COLUMN session_id VARBINARY(128) NULL AFTER namespace, ADD INDEX messages_namespace_session_sequence (namespace, session_id, sequence)",
        "CREATE TABLE context_summaries (summary_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY, actor_namespace VARBINARY(1024) NOT NULL, session_id VARBINARY(128) NOT NULL, source_namespace VARBINARY(1024) NOT NULL, summary_namespace VARBINARY(1024) NOT NULL, source_view VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, source_revision CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, after_sequence BIGINT NOT NULL, through_sequence BIGINT NOT NULL, turn_id VARBINARY(128) NOT NULL, invocation_id VARBINARY(128) NOT NULL, record_format VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, summary LONGTEXT CHARACTER SET utf8mb4 NOT NULL, UNIQUE INDEX context_summary_source_range (actor_namespace, session_id, source_namespace, through_sequence))",
        "CREATE TABLE context_summary_cursors (actor_namespace VARBINARY(1024) NOT NULL, session_id VARBINARY(128) NOT NULL, source_namespace VARBINARY(1024) NOT NULL, through_sequence BIGINT NOT NULL, summary_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, source_view VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, source_revision CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, PRIMARY KEY (actor_namespace, session_id, source_namespace))",
    ],
    transform: "existing messages retain null session identity; no legacy attribution is inferred",
    postcondition: "version=5;nullable message session provenance;strict context summary and cursor tables;v4 receipts retained",
    failed_status: &[
        StatusRow {
            table: "context_summaries",
            staged: 0,
            status: "new table",
        },
        StatusRow {
            table: "context_summary_cursors",
            staged: 0,
            status: "new table",
        },
        StatusRow {
            table: "messages",
            staged: 0,
            status: "modified",
        },
    ],
};

const V6: Definition = Definition {
    from: 5,
    to: 6,
    id: "kuru.memory.compact-checkpoint-provenance.v6",
    sql: &[
        "ALTER TABLE context_summaries MODIFY COLUMN turn_id VARBINARY(128) NULL, ADD COLUMN operation_id VARBINARY(128) NULL AFTER turn_id, ADD COLUMN producer_actor_id VARBINARY(1024) NULL AFTER operation_id",
    ],
    transform: "existing context summaries retain exact turn provenance; no operation or producer is inferred",
    postcondition: "version=6;exclusive turn/operation context provenance;v5 rows and v4 usage registry retained",
    failed_status: &[StatusRow {
        table: "context_summaries",
        staged: 0,
        status: "modified",
    }],
};

const DEFINITIONS: &[Definition] = &[V2, V3, V4, V5, V6];

#[derive(Clone, Copy)]
struct Registry {
    current: i32,
    definitions: &'static [Definition],
}

const REGISTRY: Registry = Registry {
    current: CURRENT_VERSION,
    definitions: DEFINITIONS,
};

// The permanent usage branch owns accounting state and operation receipts,
// not conversation history. Main-only message provenance must not migrate it.
const USAGE_REGISTRY: Registry = Registry {
    current: USAGE_CURRENT_VERSION,
    definitions: &[V2, V3, V4],
};

#[cfg(test)]
fn definition(to: i32) -> Result<&'static Definition> {
    REGISTRY.definition(to)
}

impl Registry {
    fn definition(self, to: i32) -> Result<&'static Definition> {
        self.validate()?;
        self.definitions
            .iter()
            .find(|definition| definition.to == to)
            .context("unsupported Dolt memory schema transition")
    }

    fn validate(self) -> Result<()> {
        ensure!(
            !self.definitions.is_empty() && self.definitions.len() <= DEFINITION_LIMIT,
            "compiled Dolt migration registry has an invalid size"
        );
        ensure!(
            self.definitions
                .last()
                .is_some_and(|definition| definition.to == self.current),
            "compiled Dolt migration registry does not end at current schema"
        );
        let mut ids = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for (index, definition) in self.definitions.iter().enumerate() {
            let expected_from = i32::try_from(index)? + 1;
            ensure!(
                definition.from == expected_from && definition.to == expected_from + 1,
                "compiled Dolt migration registry is not consecutive"
            );
            ensure!(
                ids.insert(definition.id) && targets.insert(definition.to),
                "compiled Dolt migration registry contains a duplicate"
            );
            ensure!(
                !definition.id.is_empty()
                    && definition.id.len() <= MIGRATION_ID_LIMIT
                    && definition.id.is_ascii()
                    && !definition.id.contains('\0'),
                "invalid compiled Dolt memory migration definition"
            );
            for value in [definition.transform, definition.postcondition] {
                ensure!(
                    !value.is_empty()
                        && value.len() <= FIELD_LIMIT
                        && value.is_ascii()
                        && !value.contains('\0'),
                    "invalid compiled Dolt memory migration definition"
                );
            }
            ensure!(
                !definition.sql.is_empty() && definition.sql.len() <= DEFINITION_LIMIT,
                "invalid compiled Dolt migration SQL"
            );
            for statement in definition.sql {
                ensure!(
                    !statement.is_empty()
                        && statement.len() <= 32 * 1024
                        && !statement.contains('\0'),
                    "invalid compiled Dolt migration SQL"
                );
            }
            ensure!(
                definition.failed_status.len() <= DEFINITION_LIMIT
                    && definition
                        .failed_status
                        .windows(2)
                        .all(|rows| rows[0] < rows[1]),
                "invalid compiled Dolt migration failed-status definition"
            );
            for row in definition.failed_status {
                ensure!(
                    !row.table.is_empty()
                        && row.table.len() <= FIELD_LIMIT
                        && row.table.is_ascii()
                        && !row.table.contains('\0')
                        && !row.status.is_empty()
                        && row.status.len() <= FIELD_LIMIT
                        && row.status.is_ascii()
                        && !row.status.contains('\0')
                        && matches!(row.staged, 0 | 1),
                    "invalid compiled Dolt migration failed-status definition"
                );
            }
        }
        Ok(())
    }
}

fn digest(definition: &Definition) -> String {
    // Domain separation prevents a receipt digest from being confused with a
    // hash of a user value or another Kuru durable record.
    let mut hash = Sha256::new();
    hash_field(&mut hash, b"protocol", RECEIPT_PROTOCOL.as_bytes());
    hash_field(&mut hash, b"from", &definition.from.to_be_bytes());
    hash_field(&mut hash, b"to", &definition.to.to_be_bytes());
    hash_field(&mut hash, b"id", definition.id.as_bytes());
    hash_field(
        &mut hash,
        b"sql-count",
        &u64::try_from(definition.sql.len())
            .expect("bounded definition count")
            .to_be_bytes(),
    );
    for statement in definition.sql {
        hash_field(&mut hash, b"sql", statement.as_bytes());
    }
    hash_field(&mut hash, b"transform", definition.transform.as_bytes());
    hash_field(
        &mut hash,
        b"postcondition",
        definition.postcondition.as_bytes(),
    );
    hash_field(
        &mut hash,
        b"failed-status-count",
        &u64::try_from(definition.failed_status.len())
            .expect("bounded failed-status count")
            .to_be_bytes(),
    );
    for row in definition.failed_status {
        hash_field(&mut hash, b"failed-table", row.table.as_bytes());
        hash_field(&mut hash, b"failed-staged", &row.staged.to_be_bytes());
        hash_field(&mut hash, b"failed-status", row.status.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_field(hash: &mut Sha256, tag: &[u8], value: &[u8]) {
    hash.update(
        u64::try_from(tag.len())
            .expect("static digest tag length")
            .to_be_bytes(),
    );
    hash.update(tag);
    hash.update(
        u64::try_from(value.len())
            .expect("bounded digest field length")
            .to_be_bytes(),
    );
    hash.update(value);
}

pub(super) async fn version(pool: &MySqlPool) -> Result<i32> {
    version_from_query(pool, "SELECT id, version FROM kuru_schema LIMIT 2").await
}

async fn version_from_query(pool: &MySqlPool, query: &'static str) -> Result<i32> {
    let rows = bounded_query(sqlx::query(query).fetch_all(pool))
        .await
        .context("database is not an initialized Kuru memory store")?;
    ensure!(
        rows.len() == 1,
        "Dolt memory schema must contain one stable version row"
    );
    let id: i32 = rows[0].try_get("id")?;
    ensure!(
        id == 1,
        "Dolt memory schema version row has an invalid identity"
    );
    let version: i32 = rows[0].try_get("version")?;
    ensure!(version >= 1, "invalid Dolt memory schema version {version}");
    Ok(version)
}

pub(super) async fn validate_supported(pool: &MySqlPool) -> Result<i32> {
    validate_supported_with(REGISTRY, pool).await
}

/// Validate a historical branch through its own version dispatcher.
/// This never migrates the branch or applies latest-main validation.
pub(super) async fn validate_historical(pool: &MySqlPool) -> Result<i32> {
    let version = validate_supported_with(REGISTRY, pool).await?;
    authority_working_set(pool).await?;
    Ok(version)
}

async fn validate_supported_with(registry: Registry, pool: &MySqlPool) -> Result<i32> {
    registry.validate()?;
    let found = version(pool).await?;
    validate_version_with(registry, pool, found).await?;
    Ok(found)
}

async fn validate_version_with(registry: Registry, pool: &MySqlPool, expected: i32) -> Result<()> {
    let found = version(pool).await?;
    ensure!(
        found == expected,
        "Dolt migration branch has schema version {found}, expected {expected}"
    );
    validate_schema_with(registry, pool, expected).await
}

async fn validate_schema_with(registry: Registry, pool: &MySqlPool, found: i32) -> Result<()> {
    ensure!(
        (1..=registry.current).contains(&found),
        "unsupported Dolt memory schema version {found}"
    );
    validate_schema_v1(pool).await?;
    if found == 1 {
        let receipt_tables: i64 = bounded_query(
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'kuru_migrations'",
            )
            .fetch_one(pool),
        )
        .await?;
        ensure!(
            receipt_tables == 0,
            "Dolt memory schema 1 must not contain migration receipt authority"
        );
    }
    if found >= 2 {
        validate_receipts(registry, pool, found).await?;
    }
    if found >= 3 {
        bounded_query(sqlx::query("SELECT content_format FROM messages LIMIT 0").fetch_all(pool))
            .await?;
        let columns = bounded_query(
            sqlx::query("SELECT data_type, is_nullable, column_default, character_maximum_length, character_set_name, collation_name FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'messages' AND BINARY column_name = BINARY 'content_format'")
                .fetch_all(pool),
        )
        .await?;
        ensure!(
            columns.len() == 1,
            "typed message format column is missing or ambiguous"
        );
        let column = &columns[0];
        // Dolt's information_schema preserves uppercase protocol labels for
        // some fields even when the SELECT text is lowercase.
        let default: String = column.try_get(2)?;
        ensure!(
            column
                .try_get::<String, _>(0)?
                .eq_ignore_ascii_case("varchar")
                && column.try_get::<String, _>(1)? == "NO"
                && default.trim_matches('\'') == "text-v1"
                && column.try_get::<i64, _>(3)? == 16
                && column
                    .try_get::<String, _>(4)?
                    .eq_ignore_ascii_case("ascii")
                && column
                    .try_get::<String, _>(5)?
                    .eq_ignore_ascii_case("ascii_bin"),
            "typed message format column differs from schema v3"
        );
    }
    if found >= 4 {
        validate_operation_receipt_shape(pool).await?;
    }
    if found >= 5 {
        validate_session_provenance_shape(pool, found).await?;
    }
    #[cfg(test)]
    if registry.current >= 7 && found >= 7 {
        bounded_query(
            sqlx::query("SELECT marker FROM kuru_migration_test_v7 LIMIT 0").fetch_all(pool),
        )
        .await?;
    }
    Ok(())
}

async fn validate_session_provenance_shape(pool: &MySqlPool, version: i32) -> Result<()> {
    bounded_query(sqlx::query("SELECT session_id FROM messages LIMIT 0").fetch_all(pool)).await?;
    let columns = bounded_query(
        sqlx::query("SELECT data_type, is_nullable, character_maximum_length FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'messages' AND BINARY column_name = BINARY 'session_id'")
            .fetch_all(pool),
    )
    .await?;
    ensure!(
        columns.len() == 1
            && columns[0]
                .try_get::<String, _>(0)?
                .eq_ignore_ascii_case("varbinary")
            && columns[0].try_get::<String, _>(1)? == "YES"
            && columns[0].try_get::<i64, _>(2)? == 128,
        "message session identity column differs from schema v5"
    );
    let summary_projection = if version >= 6 {
        "SELECT summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, operation_id, producer_actor_id, invocation_id, record_format, summary FROM context_summaries LIMIT 0"
    } else {
        "SELECT summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, invocation_id, record_format, summary FROM context_summaries LIMIT 0"
    };
    bounded_query(sqlx::query(summary_projection).fetch_all(pool)).await?;
    bounded_query(
        sqlx::query("SELECT actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors LIMIT 0")
            .fetch_all(pool),
    )
    .await?;
    let mut summary_columns = vec![
        ("summary_id", "char", Some(64), false),
        ("actor_namespace", "varbinary", Some(1024), false),
        ("session_id", "varbinary", Some(128), false),
        ("source_namespace", "varbinary", Some(1024), false),
        ("summary_namespace", "varbinary", Some(1024), false),
        ("source_view", "varchar", Some(64), false),
        ("source_revision", "char", Some(64), false),
        ("after_sequence", "bigint", None, false),
        ("through_sequence", "bigint", None, false),
        ("turn_id", "varbinary", Some(128), version >= 6),
    ];
    if version >= 6 {
        summary_columns.extend([
            ("operation_id", "varbinary", Some(128), true),
            ("producer_actor_id", "varbinary", Some(1024), true),
        ]);
    }
    summary_columns.extend([
        ("invocation_id", "varbinary", Some(128), false),
        ("record_format", "varchar", Some(32), false),
        ("summary", "longtext", None, false),
    ]);
    validate_columns(pool, "context_summaries", &summary_columns).await?;
    validate_columns(
        pool,
        "context_summary_cursors",
        &[
            ("actor_namespace", "varbinary", Some(1024), false),
            ("session_id", "varbinary", Some(128), false),
            ("source_namespace", "varbinary", Some(1024), false),
            ("through_sequence", "bigint", None, false),
            ("summary_id", "char", Some(64), false),
            ("source_view", "varchar", Some(64), false),
            ("source_revision", "char", Some(64), false),
        ],
    )
    .await?;
    validate_index(
        pool,
        "messages",
        "messages_namespace_session_sequence",
        false,
        &["namespace", "session_id", "sequence"],
    )
    .await?;
    validate_index(pool, "context_summaries", "PRIMARY", true, &["summary_id"]).await?;
    validate_index(
        pool,
        "context_summaries",
        "context_summary_source_range",
        true,
        &[
            "actor_namespace",
            "session_id",
            "source_namespace",
            "through_sequence",
        ],
    )
    .await?;
    validate_index(
        pool,
        "context_summary_cursors",
        "PRIMARY",
        true,
        &["actor_namespace", "session_id", "source_namespace"],
    )
    .await?;
    Ok(())
}

async fn validate_columns(
    pool: &MySqlPool,
    table: &str,
    expected: &[(&str, &str, Option<i64>, bool)],
) -> Result<()> {
    let rows = bounded_query(
        sqlx::query("SELECT column_name, data_type, is_nullable, character_maximum_length FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY ? ORDER BY ordinal_position")
            .bind(table)
            .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.len() == expected.len(),
        "{table} columns differ from schema v5"
    );
    for (row, (name, data_type, length, nullable)) in rows.into_iter().zip(expected) {
        // Dolt may preserve uppercase protocol labels for information-schema
        // projections, so use the checked SELECT order rather than label case.
        let observed_name: String = row.try_get(0)?;
        let observed_type: String = row.try_get(1)?;
        let observed_nullable: String = row.try_get(2)?;
        let observed_length: Option<i64> = row.try_get(3)?;
        ensure!(
            observed_name == *name
                && observed_type.eq_ignore_ascii_case(data_type)
                && observed_nullable == if *nullable { "YES" } else { "NO" }
                && length.is_none_or(|length| observed_length == Some(length)),
            "{table} column {name} differs from schema v5"
        );
    }
    Ok(())
}

async fn validate_index(
    pool: &MySqlPool,
    table: &str,
    index: &str,
    unique: bool,
    expected_columns: &[&str],
) -> Result<()> {
    let rows = bounded_query(
        sqlx::query("SELECT column_name, non_unique FROM information_schema.statistics WHERE table_schema = DATABASE() AND BINARY table_name = BINARY ? AND BINARY index_name = BINARY ? ORDER BY seq_in_index")
            .bind(table)
            .bind(index)
            .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.len() == expected_columns.len(),
        "{table} index {index} differs from schema v5"
    );
    for (row, expected_column) in rows.into_iter().zip(expected_columns) {
        let column: String = row.try_get(0)?;
        let non_unique: i64 = row.try_get(1)?;
        ensure!(
            column == *expected_column && (non_unique == 0) == unique,
            "{table} index {index} differs from schema v5"
        );
    }
    Ok(())
}

async fn validate_operation_receipt_shape(pool: &MySqlPool) -> Result<()> {
    bounded_query(
        sqlx::query(
            "SELECT receipt_format, method, request_digest, result_ref FROM operations LIMIT 0",
        )
        .fetch_all(pool),
    )
    .await?;
    let rows = bounded_query(
        sqlx::query("SELECT column_name, data_type, is_nullable, column_default, character_maximum_length, character_set_name, collation_name FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'operations' AND BINARY column_name IN (BINARY 'receipt_format', BINARY 'method', BINARY 'request_digest', BINARY 'result_ref')")
            .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.len() == 4,
        "retained operation receipt columns are missing or ambiguous"
    );
    let mut seen = BTreeSet::new();
    for row in rows {
        // Dolt may return uppercase information_schema field labels even when
        // the SELECT uses lowercase names; match the selected column order.
        let name: String = row.try_get(0)?;
        ensure!(
            seen.insert(name.clone()),
            "retained operation receipt column is duplicated"
        );
        let data_type: String = row.try_get(1)?;
        let nullable: String = row.try_get(2)?;
        let default: Option<String> = row.try_get(3)?;
        let length: Option<i64> = row.try_get(4)?;
        let charset: Option<String> = row.try_get(5)?;
        let collation: Option<String> = row.try_get(6)?;
        let ascii_bin = || {
            charset
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("ascii"))
                && collation
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("ascii_bin"))
        };
        let valid = match name.as_str() {
            "receipt_format" => {
                data_type.eq_ignore_ascii_case("tinyint")
                    && nullable == "NO"
                    && default
                        .as_deref()
                        .is_some_and(|value| value.trim_matches('\'') == "0")
            }
            "method" => {
                data_type.eq_ignore_ascii_case("varchar")
                    && nullable == "YES"
                    && length == Some(64)
                    && ascii_bin()
            }
            "request_digest" => {
                data_type.eq_ignore_ascii_case("char")
                    && nullable == "YES"
                    && length == Some(64)
                    && ascii_bin()
            }
            "result_ref" => {
                data_type.eq_ignore_ascii_case("varchar")
                    && nullable == "YES"
                    && length == Some(256)
                    && ascii_bin()
            }
            _ => false,
        };
        ensure!(
            valid,
            "retained operation receipt column {name} differs from schema v4"
        );
    }
    Ok(())
}

pub(super) async fn validate_current(pool: &MySqlPool) -> Result<()> {
    validate_current_with(REGISTRY, pool).await
}

async fn validate_current_with(registry: Registry, pool: &MySqlPool) -> Result<()> {
    let found = validate_supported_with(registry, pool).await?;
    ensure!(
        found == registry.current,
        "memory schema version {found} requires writable upgrade to {}",
        registry.current
    );
    inventory_with(registry, pool).await?;
    Ok(())
}

pub(super) async fn validate_active(server: &Server, pool: &MySqlPool) -> Result<()> {
    validate_active_with(REGISTRY, server, pool).await
}

/// Validate a current-schema read-only main without treating unrelated working
/// data as a migration failure.
pub(super) async fn validate_inspection(server: &Server, pool: &MySqlPool) -> Result<()> {
    validate_current_with(REGISTRY, pool).await?;
    authority_working_set(pool).await?;
    classify_historical_attempts(REGISTRY, server, pool, REGISTRY.current).await
}

/// Validate a stopped staging database exactly at the version published in its
/// immutable ready marker. A supported older stage remains activatable, but it
/// cannot contain attempts for work its publishing binary had not completed.
pub(super) async fn validate_ready(server: &Server, pool: &MySqlPool) -> Result<i32> {
    validate_ready_with(REGISTRY, server, pool).await
}

async fn validate_ready_with(registry: Registry, server: &Server, pool: &MySqlPool) -> Result<i32> {
    let found = validate_supported_with(registry, pool).await?;
    inventory_with(registry, pool).await?;
    clean(pool).await?;
    for name in reserved_names(pool).await? {
        let (target, _) = parse_attempt(&name)?;
        ensure!(
            target <= found,
            "ready Dolt memory stage contains an attempt newer than its schema"
        );
    }
    classify_historical_attempts(registry, server, pool, found).await?;
    Ok(found)
}

async fn validate_active_with(registry: Registry, server: &Server, pool: &MySqlPool) -> Result<()> {
    validate_current_with(registry, pool).await?;
    clean(pool).await?;
    classify_historical_attempts(registry, server, pool, registry.current).await
}

async fn validate_receipts(registry: Registry, pool: &MySqlPool, found: i32) -> Result<()> {
    let expected: Vec<_> = registry
        .definitions
        .iter()
        .filter(|definition| definition.to <= found)
        .collect();
    let limit = i64::try_from(expected.len() + 1)?;
    let rows = bounded_query(
        sqlx::query(
            "SELECT version, id, digest, operation FROM kuru_migrations ORDER BY version LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.len() == expected.len(),
        "Dolt memory migration receipt chain is incomplete or has extra entries"
    );
    let mut operations = BTreeSet::new();
    for (row, definition) in rows.iter().zip(expected) {
        let version: i32 = row.try_get("version")?;
        let id: String = row.try_get("id")?;
        let observed: String = row.try_get("digest")?;
        let operation: String = row.try_get("operation")?;
        ensure!(
            version == definition.to,
            "Dolt memory migration receipt version is out of order"
        );
        ensure!(
            id == definition.id,
            "Dolt memory migration receipt ID differs"
        );
        ensure!(
            observed == digest(definition),
            "Dolt memory migration receipt definition differs"
        );
        let parsed = Uuid::parse_str(&operation)
            .context("Dolt memory migration receipt UUID is malformed")?;
        ensure!(
            parsed.hyphenated().to_string() == operation,
            "Dolt memory migration receipt UUID must use lowercase canonical form"
        );
        ensure!(
            operations.insert(operation),
            "Dolt memory migration receipt operation is repeated"
        );
    }
    Ok(())
}

#[cfg(test)]
fn attempt_name(to: i32, operation: Uuid) -> String {
    attempt_name_in(RESERVED_PREFIX, to, operation)
}

fn parse_attempt(name: &str) -> Result<(i32, Uuid)> {
    parse_attempt_in(RESERVED_PREFIX, name)
}

fn attempt_name_in(prefix: &str, to: i32, operation: Uuid) -> String {
    format!("{prefix}v{to:010}_{}", operation.simple())
}

fn parse_attempt_in(prefix: &str, name: &str) -> Result<(i32, Uuid)> {
    let rest = name
        .strip_prefix(prefix)
        .context("reserved migration branch is malformed")?;
    let (version, operation) = rest
        .strip_prefix('v')
        .and_then(|rest| rest.split_once('_'))
        .context("reserved migration branch is malformed")?;
    ensure!(
        version.len() == 10 && version.bytes().all(|byte| byte.is_ascii_digit()),
        "reserved migration branch is malformed"
    );
    let version: i32 = version
        .parse()
        .context("reserved migration branch has invalid version")?;
    let operation =
        Uuid::parse_str(operation).context("reserved migration branch has invalid UUID")?;
    ensure!(
        operation.simple().to_string() == name.rsplit_once('_').expect("split above").1,
        "reserved migration branch UUID must use lowercase simple form"
    );
    Ok((version, operation))
}

async fn clean(pool: &MySqlPool) -> Result<()> {
    let changes: i64 =
        bounded_query(sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(pool))
            .await?;
    ensure!(
        changes == 0,
        "Dolt migration branch has uncommitted changes"
    );
    Ok(())
}

/// Read-only compatibility may observe ordinary uncommitted data, but schema
/// and receipt authority must always remain committed and unchanged.
async fn authority_working_set(pool: &MySqlPool) -> Result<()> {
    let rows = bounded_query(
        sqlx::query(
            "SELECT table_name, staged, status FROM dolt_status WHERE BINARY table_name = BINARY 'kuru_schema' OR BINARY table_name = BINARY 'kuru_migrations' ORDER BY BINARY table_name, staged, BINARY status LIMIT 3",
        )
        .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.is_empty(),
        "Dolt memory schema or migration receipt authority has uncommitted changes"
    );
    Ok(())
}

async fn retained_failed_shape(pool: &MySqlPool, definition: &Definition) -> Result<bool> {
    let limit = i64::try_from(definition.failed_status.len() + 1)?;
    let rows = bounded_query(
        sqlx::query(
            "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.len() == definition.failed_status.len(),
        "Dolt migration failed-status inventory is incomplete or excessive"
    );
    for (row, expected) in rows.iter().zip(definition.failed_status) {
        ensure!(
            row.try_get::<String, _>("table_name")? == expected.table
                && row.try_get::<i64, _>("staged")? == expected.staged
                && row.try_get::<String, _>("status")? == expected.status,
            "Dolt migration failed-status inventory differs from its definition"
        );
    }
    Ok(true)
}

async fn inventory_with(registry: Registry, pool: &MySqlPool) -> Result<()> {
    inventory_in(registry, pool, RESERVED_PREFIX).await
}

async fn inventory_in(registry: Registry, pool: &MySqlPool, prefix: &str) -> Result<()> {
    let names = reserved_names_in(pool, prefix).await?;
    ensure!(
        names.len() <= INVENTORY_LIMIT,
        "too many retained Dolt migration attempts"
    );
    for name in names {
        let (target, _) = parse_attempt_in(prefix, &name)?;
        registry.definition(target)?;
    }
    Ok(())
}

async fn reserved_names(pool: &MySqlPool) -> Result<Vec<String>> {
    reserved_names_in(pool, RESERVED_PREFIX).await
}

async fn reserved_names_in(pool: &MySqlPool, prefix: &str) -> Result<Vec<String>> {
    // Do not use LIKE: underscores in the namespace are wildcards there. The
    // bounded SQL-side prefix comparison keeps arbitrary user refs out of the
    // reserved inventory and caps allocation before parsing.
    let names: Vec<String> = bounded_query(
        sqlx::query_scalar(
            "SELECT name FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? LIMIT 65",
        )
        .bind(i64::try_from(prefix.len())?)
        .bind(prefix)
        .fetch_all(pool),
    )
    .await?;
    Ok(names)
}

async fn close_routed_pool(pool: Option<MySqlPool>) -> Result<()> {
    if let Some(pool) = pool {
        tokio::time::timeout(QUERY_TIMEOUT, pool.close())
            .await
            .context("migration fixture connection-pool close deadline exceeded")?;
    }
    Ok(())
}

async fn close_branch_pool(pool: &MySqlPool) -> Result<()> {
    tokio::time::timeout(QUERY_TIMEOUT, pool.close())
        .await
        .context("Dolt migration branch pool close deadline exceeded")
}

fn after_cleanup<T>(result: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "Dolt migration branch cleanup also failed: {cleanup:#}"
        ))),
    }
}

/// Build each missing schema version on its own exact-base branch.  The caller
/// retains server and startup-lock ownership until this returns and all pools
/// have been closed, so an accepted attempt cannot be abandoned by cancellation.
pub(super) async fn upgrade(server: &Server, main: &MySqlPool) -> Result<()> {
    upgrade_with(REGISTRY, server, main, &MigrationRunnerHooks::none()).await
}

#[cfg(test)]
pub(super) async fn upgrade_main_to_v3_fixture(server: &Server, main: &MySqlPool) -> Result<()> {
    const RELEASED_V3: Registry = Registry {
        current: 3,
        definitions: &[V2, V3],
    };
    upgrade_with(RELEASED_V3, server, main, &MigrationRunnerHooks::none()).await
}

/// The permanent usage branch is writable independently of main. Give its
/// staged attempts a separate owned namespace so a main migration attempt can never
/// be mistaken for an exact-base usage attempt (or vice versa).
pub(super) async fn upgrade_usage(server: &Server, usage: &MySqlPool) -> Result<()> {
    upgrade_in(
        USAGE_REGISTRY,
        server,
        usage,
        &MigrationRunnerHooks::none(),
        USAGE_RESERVED_PREFIX,
    )
    .await
}

#[cfg(test)]
pub(super) async fn upgrade_usage_with_hooks(
    server: &Server,
    usage: &MySqlPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_in(USAGE_REGISTRY, server, usage, hooks, USAGE_RESERVED_PREFIX).await
}

pub(super) async fn validate_usage(server: &Server, usage: &MySqlPool) -> Result<()> {
    let found = validate_supported_with(USAGE_REGISTRY, usage).await?;
    ensure!(
        found == USAGE_REGISTRY.current,
        "usage branch schema version {found} requires writable upgrade to {}",
        USAGE_REGISTRY.current
    );
    clean(usage).await?;
    inventory_in(USAGE_REGISTRY, usage, USAGE_RESERVED_PREFIX).await?;
    classify_historical_attempts_in(
        USAGE_REGISTRY,
        server,
        usage,
        USAGE_REGISTRY.current,
        USAGE_RESERVED_PREFIX,
    )
    .await
}

#[cfg(test)]
pub(super) async fn upgrade_with_hooks(
    server: &Server,
    main: &MySqlPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_with(REGISTRY, server, main, hooks).await
}

async fn upgrade_with(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_in(registry, server, main, hooks, RESERVED_PREFIX).await
}

async fn upgrade_in(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    hooks: &MigrationRunnerHooks,
    prefix: &str,
) -> Result<()> {
    loop {
        let found = validate_supported_with(registry, main).await?;
        inventory_in(registry, main, prefix).await?;
        classify_historical_attempts_in(registry, server, main, found, prefix).await?;
        if found == registry.current {
            return Ok(());
        }
        ensure!(
            found < registry.current,
            "unsupported Dolt memory schema version {found}"
        );
        let definition = registry.definition(found + 1)?;
        let base = revision(main).await?;
        clean(main).await?;
        if prefix == RESERVED_PREFIX && definition.to == V5.to {
            ensure_usage_branch_at_v4(main, &base).await?;
        }
        let (branch, operation, ready) =
            discover_current_attempt_in(registry, server, main, definition, &base, hooks, prefix)
                .await?;
        if ready {
            let attempt = server.pool(&branch).await?;
            let inspected = async {
                let target = revision(&attempt).await?;
                validate_attempt(registry, &attempt, definition, operation, &base).await?;
                Ok(target)
            }
            .await;
            let target = after_cleanup(inspected, close_branch_pool(&attempt).await)?;
            publish(registry, main, definition, &branch, &base, &target, hooks).await?;
            continue;
        }
        let attempt = server.pool(&branch).await?;
        let built = async {
            hooks.reach(MigrationBoundary::BeforeDdl).await?;
            build_attempt(registry, &attempt, definition, operation, hooks).await?;
            let target = revision(&attempt).await?;
            validate_attempt(registry, &attempt, definition, operation, &base).await?;
            Ok(target)
        }
        .await;
        let target = after_cleanup(built, close_branch_pool(&attempt).await)?;
        publish(registry, main, definition, &branch, &base, &target, hooks).await?;
    }
}

async fn ensure_usage_branch_at_v4(main: &MySqlPool, base: &str) -> Result<()> {
    let branches: Vec<String> = bounded_query(
        sqlx::query_scalar("SELECT name FROM dolt_branches WHERE BINARY name = BINARY ? LIMIT 2")
            .bind(super::usage_ledger::BRANCH)
            .fetch_all(main),
    )
    .await?;
    ensure!(
        branches.len() <= 1,
        "usage ledger branch identity is ambiguous"
    );
    if branches.is_empty() {
        // Schema 5 is main-only session provenance. Anchor a new permanent
        // usage ledger at the exact clean schema-4 head before main advances,
        // so later establishment never inherits or fabricates schema 5.
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(super::usage_ledger::BRANCH)
                .bind(base)
                .fetch_all(main),
        )
        .await?;
    }
    Ok(())
}

async fn classify_historical_attempts(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    current: i32,
) -> Result<()> {
    classify_historical_attempts_in(registry, server, main, current, RESERVED_PREFIX).await
}

async fn classify_historical_attempts_in(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    current: i32,
    prefix: &str,
) -> Result<()> {
    let main_head = revision(main).await?;
    for name in reserved_names_in(main, prefix).await? {
        let (target, operation) = parse_attempt_in(prefix, &name)?;
        ensure!(
            target <= current + 1,
            "reserved Dolt migration branch targets an out-of-order step"
        );
        if target > current {
            continue;
        }
        let definition = registry.definition(target)?;
        let attempt = server.pool(&name).await?;
        let inspected = async {
            let head = revision(&attempt).await?;
            let head_version: i32 = bounded_query(
                sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
                    .fetch_one(attempt.as_ref()),
            )
            .await?;
            let dirty: i64 = bounded_query(
                sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(attempt.as_ref()),
            )
            .await?;
            let outcome = if dirty == 0 {
                validate_version_with(registry, &attempt, definition.to).await?;
                let receipt: String = bounded_query(
                    sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = ?")
                        .bind(definition.to)
                        .fetch_one(attempt.as_ref()),
                )
                .await?;
                ensure!(
                    receipt == operation.hyphenated().to_string(),
                    "historical Dolt migration receipt does not match its branch"
                );
                let parent = sole_parent(&attempt, &head).await?;
                validate_commit_version(registry, server, &parent, definition.from).await?;
                ancestor(main, &head).await? && ancestor(main, &parent).await?
            } else {
                ensure!(
                    head_version == definition.from,
                    "dirty historical Dolt migration branch has an unexpected schema"
                );
                validate_commit_version(registry, server, &head, definition.from).await?;
                ensure!(
                    retained_failed_shape(&attempt, definition).await?,
                    "dirty historical Dolt migration branch has an unexpected working set"
                );
                ancestor(main, &head).await?
            };
            Ok((head, outcome))
        }
        .await;
        let (head, outcome) = after_cleanup(inspected, close_branch_pool(&attempt).await)?;
        ensure!(
            outcome,
            "historical Dolt migration branch is not retained by active history"
        );
        ensure!(
            head != main_head || target == current,
            "historical Dolt migration branch unexpectedly names active main"
        );
    }
    Ok(())
}

async fn ancestor(main: &MySqlPool, ancestor: &str) -> Result<bool> {
    let count: i64 = bounded_query(
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?")
            .bind(ancestor)
            .fetch_one(main),
    )
    .await?;
    Ok(count == 1)
}

async fn sole_parent(pool: &MySqlPool, commit: &str) -> Result<String> {
    let parents: Vec<String> = bounded_query(
        sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index LIMIT 2",
        )
        .bind(commit)
        .fetch_all(pool),
    )
    .await?;
    ensure!(
        parents.len() == 1,
        "Dolt migration target does not have exactly one parent"
    );
    Ok(parents.into_iter().next().expect("one parent checked"))
}

async fn validate_commit_version(
    registry: Registry,
    server: &Server,
    commit: &str,
    expected: i32,
) -> Result<()> {
    let pool = server.pool(commit).await?;
    let validation = validate_version_with(registry, &pool, expected).await;
    after_cleanup(validation, close_branch_pool(&pool).await)
}

/// Return the sole pristine branch for this exact base, or create one.  A
/// completed branch is returned as `ready`; retained dirty attempts are
/// evidence, never a reset/reuse target.  Every other shape is ambiguous.
async fn discover_current_attempt_in(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    definition: &Definition,
    base: &str,
    hooks: &MigrationRunnerHooks,
    prefix: &str,
) -> Result<(String, Uuid, bool)> {
    let names = reserved_names_in(main, prefix).await?;
    let inventory_len = names.len();
    let mut reusable = None;
    for name in names {
        let (target, operation) = parse_attempt_in(prefix, &name)?;
        if target != definition.to {
            continue;
        }
        let attempt = server.pool(&name).await?;
        let inspected = async {
            let head = revision(&attempt).await?;
            let dirty = bounded_query(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
                    .fetch_one(attempt.as_ref()),
            )
            .await?;
            let head_version: i32 = bounded_query(
                sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
                    .fetch_one(attempt.as_ref()),
            )
            .await?;
            if head == base && dirty == 0 && head_version == definition.from {
                validate_version_with(registry, &attempt, definition.from).await?;
                Ok(Some(false)) // pristine
            } else if dirty == 0 && head_version == definition.to {
                validate_attempt(registry, &attempt, definition, operation, base).await?;
                Ok(Some(true))
            } else if head == base && dirty > 0 && head_version == definition.from {
                validate_commit_version(registry, server, &head, definition.from).await?;
                ensure!(
                    retained_failed_shape(&attempt, definition).await?,
                    "reserved Dolt migration branch has an unexpected failed working set"
                );
                Ok(None) // retained failed attempt
            } else {
                bail!("reserved Dolt migration branch has an unresolved state");
            }
        }
        .await;
        let shape = after_cleanup(inspected, close_branch_pool(&attempt).await)?;
        if let Some(ready) = shape {
            ensure!(
                reusable.is_none(),
                "multiple publishable Dolt migration attempts exist"
            );
            reusable = Some((name, operation, ready));
        }
    }
    if let Some(attempt) = reusable {
        return Ok(attempt);
    }
    ensure!(
        inventory_len < INVENTORY_LIMIT,
        "retained Dolt migration attempts leave no capacity for a fresh attempt"
    );
    let operation = Uuid::new_v4();
    let branch = attempt_name_in(prefix, definition.to, operation);
    hooks.describe(MigrationBoundary::BeforeBranch, &branch, None);
    let routed = routed_pool(main, hooks, MigrationBoundary::BeforeBranch).await?;
    let command_pool = routed.as_ref().unwrap_or(main);
    let (mut connection, connection_id) = super::owned_connection(command_pool).await?;
    let created = async {
        hooks.reach(MigrationBoundary::BeforeBranch).await?;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&branch)
                .bind(base)
                .fetch_all(&mut connection),
        )
        .await
    }
    .await;
    drop(connection);
    let close_result = close_routed_pool(routed).await;
    let session_result = super::await_session_end(main, connection_id, QUERY_TIMEOUT).await;
    if let Err(cleanup) = after_cleanup(close_result, session_result) {
        return match created {
            Ok(_) => Err(cleanup),
            Err(error) => Err(error.context(format!(
                "Dolt migration branch-create cleanup also failed: {cleanup:#}"
            ))),
        };
    }
    let observed: Option<String> = bounded_query(
        sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE name = ?")
            .bind(&branch)
            .fetch_optional(main),
    )
    .await?;
    match (created, observed) {
        (Ok(_), Some(observed)) | (Err(_), Some(observed)) if observed == base => {
            Ok((branch, operation, false))
        }
        (Ok(_), Some(_)) | (Err(_), Some(_)) => {
            bail!("new Dolt migration branch did not retain its exact base")
        }
        (Ok(_), None) => bail!("Dolt did not retain a created migration branch"),
        (Err(error), None) => Err(error).context("create isolated Dolt migration branch"),
    }
}

async fn build_attempt(
    registry: Registry,
    pool: &MySqlPool,
    definition: &Definition,
    operation: Uuid,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    let routed = routed_pool(pool, hooks, MigrationBoundary::BeforeCommit).await?;
    let command_pool = routed.as_ref().unwrap_or(pool);
    let (mut connection, id) = super::owned_connection(command_pool).await?;
    let result = async {
        for statement in definition.sql {
            bounded_query(sqlx::query(*statement).execute(&mut connection)).await?;
        }
        hooks.reach(MigrationBoundary::AfterDdl).await?;
        bounded_query(sqlx::query("START TRANSACTION").execute(&mut connection)).await?;
        let advanced = bounded_query(
            sqlx::query("UPDATE kuru_schema SET version = ? WHERE id = 1 AND version = ?")
                .bind(definition.to)
                .bind(definition.from)
                .execute(&mut connection),
        )
        .await?;
        ensure!(
            advanced.rows_affected() == 1,
            "Dolt migration schema version did not advance exactly once"
        );
        bounded_query(
            sqlx::query(
                "INSERT INTO kuru_migrations (version, id, digest, operation) VALUES (?, ?, ?, ?)",
            )
            .bind(definition.to)
            .bind(definition.id)
            .bind(digest(definition))
            .bind(operation.hyphenated().to_string())
            .execute(&mut connection),
        )
        .await?;
        hooks.reach(MigrationBoundary::BeforeCommit).await?;
        bounded_query(
            sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                .bind(format!(
                    "Upgrade Kuru memory schema {} [{}]",
                    definition.to, operation
                ))
                .bind(AUTHOR)
                .fetch_all(&mut connection),
        )
        .await?;
        bounded_query(sqlx::query("COMMIT").execute(&mut connection)).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    drop(connection);
    let close_result = close_routed_pool(routed).await;
    let session_result = super::await_session_end(pool, id, QUERY_TIMEOUT).await;
    if let Err(cleanup) = after_cleanup(close_result, session_result) {
        return after_cleanup(result, Err(cleanup));
    }
    if result.is_ok() {
        return Ok(());
    }
    // A dropped reply after DOLT_COMMIT must be reconciled from the branch's
    // durable head and receipt, never replayed on a second branch.
    if validate_version_with(registry, pool, definition.to)
        .await
        .is_ok()
    {
        return Ok(());
    }
    result.context("Dolt migration commit failed")
}

async fn validate_attempt(
    registry: Registry,
    pool: &MySqlPool,
    definition: &Definition,
    operation: Uuid,
    base: &str,
) -> Result<()> {
    clean(pool).await?;
    validate_version_with(registry, pool, definition.to).await?;
    let actual: String = bounded_query(
        sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = ?")
            .bind(definition.to)
            .fetch_one(pool),
    )
    .await?;
    ensure!(
        actual == operation.hyphenated().to_string(),
        "Dolt migration branch receipt does not match its name"
    );
    let target = revision(pool).await?;
    let parent = sole_parent(pool, &target).await?;
    ensure!(
        parent == base,
        "Dolt migration target does not have its exact sole base parent"
    );
    Ok(())
}

async fn publish(
    registry: Registry,
    main: &MySqlPool,
    definition: &Definition,
    branch: &str,
    base: &str,
    target: &str,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    ensure!(
        revision(main).await? == base,
        "Dolt main changed before migration publication"
    );
    clean(main).await?;
    validate_version_with(registry, main, definition.from).await?;
    hooks.describe(MigrationBoundary::BeforePublish, branch, Some(target));
    let routed = routed_pool(main, hooks, MigrationBoundary::BeforePublish).await?;
    let command_pool = routed.as_ref().unwrap_or(main);
    let (mut connection, id) = super::owned_connection(command_pool).await?;
    let result = async {
        hooks.reach(MigrationBoundary::BeforePublish).await?;
        bounded_query(
            sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                .bind(branch)
                .fetch_all(&mut connection),
        )
        .await
    }
    .await;
    drop(connection);
    let close_result = close_routed_pool(routed).await;
    let session_result = super::await_session_end(main, id, QUERY_TIMEOUT).await;
    if let Err(cleanup) = after_cleanup(close_result, session_result) {
        return after_cleanup(result.map(|_| ()), Err(cleanup));
    }
    let observed = revision(main).await?;
    ensure!(
        observed == base || observed == target,
        "cannot reconcile Dolt migration publication: main diverged"
    );
    if observed == target {
        clean(main).await?;
        validate_version_with(registry, main, definition.to).await?;
        return Ok(());
    }
    clean(main).await?;
    result.context("Dolt migration fast-forward failed")?;
    bail!("Dolt migration did not fast-forward to its validated target")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Eq, PartialEq)]
    struct DurableSnapshot {
        head: String,
        refs: Vec<(String, String)>,
        status: Vec<(String, i64, String)>,
    }

    const V7: Definition = Definition {
        from: 6,
        to: 7,
        id: "kuru.memory.test-marker.v7",
        sql: &["CREATE TABLE kuru_migration_test_v7 (marker INT PRIMARY KEY)"],
        transform: "none",
        postcondition: "version=7;test marker table exists;v6 compact provenance remains exact",
        failed_status: &[StatusRow {
            table: "kuru_migration_test_v7",
            staged: 0,
            status: "new table",
        }],
    };
    const TEST_DEFINITIONS: &[Definition] = &[V2, super::V3, super::V4, super::V5, super::V6, V7];
    const TEST_REGISTRY: Registry = Registry {
        current: 7,
        definitions: TEST_DEFINITIONS,
    };
    const RELEASED_V3_DEFINITIONS: &[Definition] = &[V2, super::V3];
    const RELEASED_V3_REGISTRY: Registry = Registry {
        current: 3,
        definitions: RELEASED_V3_DEFINITIONS,
    };
    const RELEASED_V5_DEFINITIONS: &[Definition] = &[V2, super::V3, super::V4, super::V5];
    const RELEASED_V5_REGISTRY: Registry = Registry {
        current: 5,
        definitions: RELEASED_V5_DEFINITIONS,
    };

    const RELEASED_V4_REGISTRY: Registry = Registry {
        current: 4,
        definitions: &[V2, super::V3, super::V4],
    };

    #[tokio::test]
    async fn older_registry_rejects_v4_store_without_mutating_it() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "4".repeat(64)),
        )?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        upgrade_with(
            RELEASED_V4_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        let before = durable_snapshot(&main).await?;
        let error = validate_active_with(RELEASED_V3_REGISTRY, &server, &main)
            .await
            .expect_err("a v3 binary must reject v4 memory before opening it for writes");
        assert!(
            format!("{error:#}").contains("unsupported Dolt memory schema version 4"),
            "unexpected older-registry refusal: {error:#}"
        );
        assert_eq!(durable_snapshot(&main).await?, before);
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(())
    }

    /// Produce the exact released v2 shape without letting ordinary open
    /// immediately advance it to the current production schema.
    async fn released_v2(options: &super::super::OpenOptions) -> Result<String> {
        super::super::tests::released_v1(options).await?;
        let server = super::super::tests::released_server(options).await?;
        let main = server.pool("main").await?;
        bounded_query(sqlx::query(V2.sql[0]).execute(main.as_ref())).await?;
        bounded_query(
            sqlx::query("INSERT INTO kuru_migrations VALUES (2, ?, ?, ?)")
                .bind(V2.id)
                .bind(digest(&V2))
                .bind(Uuid::new_v4().hyphenated().to_string())
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("UPDATE kuru_schema SET version = 2 WHERE id = 1").execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Release schema v2 fixture").await?;
        let head = revision(&main).await?;
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(head)
    }

    #[tokio::test]
    async fn released_v2_json_looking_text_and_historical_views_survive_current_upgrade()
    -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "c".repeat(64)),
        )?;
        released_v2(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        let legacy = r#"{"blocks":[{"type":"tool_use","id":"literal"}]}"#;
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                .bind(b"history".as_slice())
                .bind(b"user".as_slice())
                .bind(legacy)
                .execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Retain JSON-looking literal text").await?;
        let old_head = revision(&main).await?;
        main.close().await;
        drop(main);
        server.close().await?;

        let store = super::super::MemoryStore::open(options.clone()).await?;
        assert_eq!(version(&store.pool).await?, CURRENT_VERSION);
        assert_eq!(
            store.history("history", 10).await?[0].plain_text(),
            Some(legacy)
        );
        let format: String = bounded_query(
            sqlx::query_scalar("SELECT content_format FROM messages WHERE namespace = ?")
                .bind(b"history".as_slice())
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(format, "text-v1");
        let old_pool = store.shared.server.pool(&old_head).await?;
        let old_view = super::super::MemoryStore {
            shared: store.shared.clone(),
            pool: old_pool.clone(),
            branch: old_head.clone(),
            logical_receipt: None,
        };
        assert_eq!(
            old_view.history("history", 10).await?[0].plain_text(),
            Some(legacy)
        );
        assert!(
            old_view
                .append_message("history", &kuru_core::Message::text("user", "typed"))
                .await
                .is_err()
        );

        let candidate_name = format!("candidate_{}", Uuid::new_v4().simple());
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&candidate_name)
                .bind(&old_head)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        let candidate_pool = store.shared.server.pool(&candidate_name).await?;
        let candidate = super::super::MemoryStore {
            shared: store.shared.clone(),
            pool: candidate_pool.clone(),
            branch: candidate_name,
            logical_receipt: None,
        };
        candidate.append("private/notes", "note", legacy).await?;
        assert_eq!(
            candidate.notes("private/notes", 10).await?[0].content,
            legacy
        );
        assert!(
            candidate
                .append_message("private/notes", &kuru_core::Message::text("note", "typed"))
                .await
                .is_err()
        );
        assert_eq!(version(&candidate_pool).await?, 2);

        let typed = kuru_core::Message::text("assistant", "new typed text");
        store.append_message("history", &typed).await?;
        assert_eq!(
            store.history("history", 10).await?,
            vec![kuru_core::Message::text("user", legacy), typed]
        );
        let formats: Vec<String> = bounded_query(
            sqlx::query_scalar(
                "SELECT content_format FROM messages WHERE namespace = ? ORDER BY sequence",
            )
            .bind(b"history".as_slice())
            .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(formats, ["text-v1", "typed-v1"]);
        candidate_pool.close().await;
        old_pool.close().await;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn released_v4_rows_remain_unattributed_when_session_provenance_activates() -> Result<()>
    {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "9".repeat(64)),
        )?;
        released_v2(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        upgrade_with(
            USAGE_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        let legacy_insert = bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, 'text-v1', ?)")
                .bind(b"private/actor".as_slice())
                .bind(b"user".as_slice())
                .bind("released-v4-legacy")
                .execute(main.as_ref()),
        )
        .await?;
        let legacy_sequence = i64::try_from(legacy_insert.last_insert_id())
            .context("released v4 legacy sequence exceeds integer range")?;
        let typed_insert = bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, 'typed-v1', ?)")
                .bind(b"private/actor".as_slice())
                .bind(b"assistant".as_slice())
                .bind(r#"{"blocks":[{"type":"text","text":"released-v4-typed"}]}"#)
                .execute(main.as_ref()),
        )
        .await?;
        let typed_sequence = i64::try_from(typed_insert.last_insert_id())
            .context("released v4 typed sequence exceeds integer range")?;
        commit_fixture(&main, "Released schema v4 message rows").await?;
        main.close().await;
        drop(main);
        server.close().await?;

        let store = super::super::MemoryStore::open(options).await?;
        assert_eq!(version(&store.pool).await?, CURRENT_VERSION);
        let retained: Vec<(i64, Option<Vec<u8>>, String)> = bounded_query(
            sqlx::query_as("SELECT sequence, session_id, content FROM messages WHERE namespace = ? ORDER BY sequence")
                .bind(b"private/actor".as_slice())
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(
            retained,
            vec![
                (legacy_sequence, None, "released-v4-legacy".into()),
                (
                    typed_sequence,
                    None,
                    r#"{"blocks":[{"type":"text","text":"released-v4-typed"}]}"#.into(),
                ),
            ]
        );
        store
            .append_session_message(
                "private/actor",
                "session-a",
                &kuru_core::Message::text("user", "session-a"),
            )
            .await?;
        store
            .append_session_message(
                "private/actor",
                "session-b",
                &kuru_core::Message::text("user", "session-b"),
            )
            .await?;
        let sequences: Vec<i64> = bounded_query(
            sqlx::query_scalar("SELECT sequence FROM messages ORDER BY sequence")
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            bounded_query(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM messages WHERE session_id IS NULL"
                )
                .fetch_one(store.pool.as_ref()),
            )
            .await?,
            2
        );
        store.close().await?;
        Ok(())
    }

    async fn durable_snapshot(pool: &MySqlPool) -> Result<DurableSnapshot> {
        let ref_rows = bounded_query(
            sqlx::query("SELECT name, hash FROM dolt_branches ORDER BY BINARY name LIMIT 129")
                .fetch_all(pool),
        )
        .await?;
        ensure!(
            ref_rows.len() <= 128,
            "fixture branch inventory is excessive"
        );
        let status_rows = bounded_query(
            sqlx::query(
                "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT 65",
            )
            .fetch_all(pool),
        )
        .await?;
        ensure!(
            status_rows.len() <= 64,
            "fixture status inventory is excessive"
        );
        Ok(DurableSnapshot {
            head: revision(pool).await?,
            refs: ref_rows
                .into_iter()
                .map(|row| Ok((row.try_get("name")?, row.try_get("hash")?)))
                .collect::<Result<_>>()?,
            status: status_rows
                .into_iter()
                .map(|row| {
                    Ok((
                        row.try_get("table_name")?,
                        row.try_get("staged")?,
                        row.try_get("status")?,
                    ))
                })
                .collect::<Result<_>>()?,
        })
    }

    async fn commit_fixture(pool: &MySqlPool, message: &str) -> Result<()> {
        bounded_query(
            sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                .bind(message)
                .bind(AUTHOR)
                .fetch_all(pool),
        )
        .await?;
        clean(pool).await
    }

    async fn assert_failed_runner_unchanged(
        registry: Registry,
        server: &Server,
        main: &MySqlPool,
        before: &DurableSnapshot,
        expected: &str,
    ) -> Result<()> {
        let hooks = MigrationRunnerHooks::none();
        let error = upgrade_with(registry, server, main, &hooks)
            .await
            .expect_err("invalid migration state was accepted");
        assert!(
            format!("{error:#}").contains(expected),
            "unexpected migration rejection: {error:#}"
        );
        assert_eq!(&durable_snapshot(main).await?, before);
        Ok(())
    }

    async fn snapshot_attempts(
        server: &Server,
        names: &[String],
    ) -> Result<Vec<(String, DurableSnapshot)>> {
        let mut snapshots = Vec::with_capacity(names.len());
        for name in names {
            let pool = server.pool(name).await?;
            let snapshot = durable_snapshot(&pool).await;
            let snapshot = after_cleanup(snapshot, close_branch_pool(&pool).await)?;
            snapshots.push((name.clone(), snapshot));
        }
        Ok(snapshots)
    }

    async fn assert_attempts_unchanged(
        server: &Server,
        before: Vec<(String, DurableSnapshot)>,
    ) -> Result<()> {
        for (name, expected) in before {
            let pool = server.pool(&name).await?;
            let observed = durable_snapshot(&pool).await;
            let observed = after_cleanup(observed, close_branch_pool(&pool).await)?;
            assert_eq!(observed, expected, "migration attempt {name} changed");
        }
        Ok(())
    }

    #[test]
    fn registry_definitions_and_reserved_names_are_bounded() {
        REGISTRY.validate().unwrap();
        TEST_REGISTRY.validate().unwrap();
        let definition = definition(2).unwrap();
        assert_eq!(definition.from, 1);
        assert_eq!(digest(definition).len(), 64);
        let name = attempt_name(2, Uuid::nil());
        assert_eq!(parse_attempt(&name).unwrap(), (2, Uuid::nil()));
        let future = attempt_name(6, Uuid::nil());
        assert_eq!(parse_attempt(&future).unwrap(), (6, Uuid::nil()));
        assert!(REGISTRY.definition(6).is_err());
        for invalid in [
            "kuru_migration_v2_bad",
            "kuru_migration_v0000000002_NOT-A-UUID",
            "KURU_MIGRATION_v0000000002_00000000000000000000000000000000",
            "kuru_migration_v0000000002_00000000-0000-0000-0000-000000000000",
        ] {
            assert!(parse_attempt(invalid).is_err());
        }
    }

    #[test]
    fn registry_rejects_ambiguous_or_out_of_order_definitions() {
        const GAP: Definition = Definition {
            from: 2,
            to: 3,
            ..V2
        };
        const DUPLICATE_ID: Definition = Definition {
            from: 2,
            to: 3,
            ..V2
        };
        const UNSORTED_STATUS: &[StatusRow] = &[
            StatusRow {
                table: "z",
                staged: 0,
                status: "new table",
            },
            StatusRow {
                table: "a",
                staged: 0,
                status: "new table",
            },
        ];
        const BAD_STATUS: Definition = Definition {
            failed_status: UNSORTED_STATUS,
            ..V2
        };

        assert!(
            Registry {
                current: 3,
                definitions: &[GAP],
            }
            .validate()
            .is_err()
        );
        assert!(
            Registry {
                current: 3,
                definitions: &[V2, DUPLICATE_ID],
            }
            .validate()
            .is_err()
        );
        assert!(
            Registry {
                current: 2,
                definitions: &[BAD_STATUS],
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn receipt_digest_binds_each_declared_definition_field() {
        let baseline = digest(&V2);
        for changed in [
            Definition { from: 0, ..V2 },
            Definition { to: 3, ..V2 },
            Definition {
                id: "kuru.memory.receipts.changed.v2",
                ..V2
            },
            Definition {
                sql: &["CREATE TABLE another_table (id INT PRIMARY KEY)"],
                ..V2
            },
            Definition {
                transform: "copy legacy values",
                ..V2
            },
            Definition {
                postcondition: "different validator identity",
                ..V2
            },
            Definition {
                failed_status: &[],
                ..V2
            },
            Definition {
                failed_status: &[StatusRow {
                    table: "kuru_migrations",
                    staged: 1,
                    status: "new table",
                }],
                ..V2
            },
        ] {
            assert_ne!(digest(&changed), baseline);
        }
        assert_ne!(digest(&V2), digest(&V5));
    }

    #[tokio::test]
    async fn persisted_invalid_schema_authority_is_rejected_without_mutation() -> Result<()> {
        #[derive(Clone, Copy)]
        enum Corruption {
            MissingReceipt,
            ChangedReceipt,
            ExtraReceipt,
            FutureSchema,
        }

        for (index, corruption) in [
            Corruption::MissingReceipt,
            Corruption::ChangedReceipt,
            Corruption::ExtraReceipt,
            Corruption::FutureSchema,
        ]
        .into_iter()
        .enumerate()
        {
            let root = crate::test_support::tempdir()?;
            let options = crate::test_support::open_options(
                root.path().join("private"),
                format!("project/{index:064x}"),
            )?;
            let store = crate::test_support::open_local_fixture(options.clone()).await?;
            match corruption {
                Corruption::MissingReceipt => {
                    bounded_query(
                        sqlx::query("DELETE FROM kuru_migrations WHERE version = 2")
                            .execute(store.pool.as_ref()),
                    )
                    .await?;
                }
                Corruption::ChangedReceipt => {
                    bounded_query(
                        sqlx::query("UPDATE kuru_migrations SET digest = ? WHERE version = 2")
                            .bind("0".repeat(64))
                            .execute(store.pool.as_ref()),
                    )
                    .await?;
                }
                Corruption::ExtraReceipt => {
                    bounded_query(
                        sqlx::query("INSERT INTO kuru_migrations VALUES (6, ?, ?, ?)")
                            .bind("fixture.extra.v6")
                            .bind("0".repeat(64))
                            .bind(Uuid::new_v4().hyphenated().to_string())
                            .execute(store.pool.as_ref()),
                    )
                    .await?;
                }
                Corruption::FutureSchema => {
                    bounded_query(
                        sqlx::query("UPDATE kuru_schema SET version = 99 WHERE id = 1")
                            .execute(store.pool.as_ref()),
                    )
                    .await?;
                }
            }
            commit_fixture(&store.pool, "Persist invalid schema authority fixture").await?;
            let before = durable_snapshot(&store.pool).await?;
            store.close().await?;

            let error = super::super::MemoryStore::open(options.clone())
                .await
                .expect_err("invalid persisted schema authority was accepted");
            let expected = match corruption {
                Corruption::MissingReceipt | Corruption::ExtraReceipt => {
                    "receipt chain is incomplete or has extra entries"
                }
                Corruption::ChangedReceipt => "receipt definition differs",
                Corruption::FutureSchema => "unsupported Dolt memory schema version 99",
            };
            assert!(
                format!("{error:#}").contains(expected),
                "unexpected persisted-schema rejection: {error:#}"
            );
            let server = super::super::tests::released_server(&options).await?;
            let main = server.pool("main").await?;
            assert_eq!(durable_snapshot(&main).await?, before);
            main.close().await;
            drop(main);
            server.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn migration_attempt_rejection_covers_ambiguous_ready_and_dirty_shapes() -> Result<()> {
        #[derive(Clone, Copy)]
        enum InvalidAttempt {
            AmbiguousPristine,
            AmbiguousReady,
            NameReceiptMismatch,
            UnexpectedDirty,
            NonBase,
            DirtyNonBase,
        }

        for (index, invalid) in [
            InvalidAttempt::AmbiguousPristine,
            InvalidAttempt::AmbiguousReady,
            InvalidAttempt::NameReceiptMismatch,
            InvalidAttempt::UnexpectedDirty,
            InvalidAttempt::NonBase,
            InvalidAttempt::DirtyNonBase,
        ]
        .into_iter()
        .enumerate()
        {
            let store = super::super::MemoryStore::temporary().await?;
            let base = store.revision().await?;
            let count = if matches!(
                invalid,
                InvalidAttempt::AmbiguousPristine | InvalidAttempt::AmbiguousReady
            ) {
                2
            } else {
                1
            };
            let mut attempts = Vec::new();
            for _ in 0..count {
                let operation = Uuid::new_v4();
                let name = attempt_name(6, operation);
                bounded_query(
                    sqlx::query("CALL DOLT_BRANCH(?, ?)")
                        .bind(&name)
                        .bind(&base)
                        .fetch_all(store.pool.as_ref()),
                )
                .await?;
                attempts.push((name, operation));
            }
            match invalid {
                InvalidAttempt::AmbiguousPristine => {}
                InvalidAttempt::AmbiguousReady => {
                    for (name, operation) in &attempts {
                        let attempt = store.shared.server.pool(name).await?;
                        let built = build_attempt(
                            TEST_REGISTRY,
                            &attempt,
                            &V6,
                            *operation,
                            &MigrationRunnerHooks::none(),
                        )
                        .await;
                        after_cleanup(built, close_branch_pool(&attempt).await)?;
                    }
                }
                InvalidAttempt::NameReceiptMismatch => {
                    let attempt = store.shared.server.pool(&attempts[0].0).await?;
                    let mismatched = Uuid::new_v4();
                    ensure!(mismatched != attempts[0].1, "fixture UUIDs must differ");
                    let built = build_attempt(
                        TEST_REGISTRY,
                        &attempt,
                        &V6,
                        mismatched,
                        &MigrationRunnerHooks::none(),
                    )
                    .await;
                    after_cleanup(built, close_branch_pool(&attempt).await)?;
                }
                InvalidAttempt::UnexpectedDirty => {
                    let attempt = store.shared.server.pool(&attempts[0].0).await?;
                    bounded_query(
                        sqlx::query("CREATE TABLE unrelated_fixture (id INT PRIMARY KEY)")
                            .execute(attempt.as_ref()),
                    )
                    .await?;
                    close_branch_pool(&attempt).await?;
                }
                InvalidAttempt::NonBase | InvalidAttempt::DirtyNonBase => {
                    let attempt = store.shared.server.pool(&attempts[0].0).await?;
                    bounded_query(
                        sqlx::query(
                            "INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)",
                        )
                        .bind(b"fixture".as_slice())
                        .bind(b"user".as_slice())
                        .bind(format!("non-base {index}"))
                        .execute(attempt.as_ref()),
                    )
                    .await?;
                    commit_fixture(&attempt, "Create non-base migration fixture").await?;
                    if matches!(invalid, InvalidAttempt::DirtyNonBase) {
                        bounded_query(
                            sqlx::query(
                                "CREATE TABLE unrelated_dirty_fixture (id INT PRIMARY KEY)",
                            )
                            .execute(attempt.as_ref()),
                        )
                        .await?;
                    }
                    close_branch_pool(&attempt).await?;
                }
            }
            let names: Vec<_> = attempts.iter().map(|(name, _)| name.clone()).collect();
            let attempt_before = snapshot_attempts(&store.shared.server, &names).await?;
            let before = durable_snapshot(&store.pool).await?;
            assert_failed_runner_unchanged(
                TEST_REGISTRY,
                &store.shared.server,
                &store.pool,
                &before,
                match invalid {
                    InvalidAttempt::AmbiguousPristine | InvalidAttempt::AmbiguousReady => {
                        "multiple publishable"
                    }
                    InvalidAttempt::NameReceiptMismatch => "receipt does not match its name",
                    InvalidAttempt::UnexpectedDirty => "failed-status inventory",
                    InvalidAttempt::NonBase | InvalidAttempt::DirtyNonBase => "unresolved state",
                },
            )
            .await?;
            assert_attempts_unchanged(&store.shared.server, attempt_before).await?;
            store.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn migration_attempt_rejection_preserves_exact_capacity() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "6".repeat(64)),
        )?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;

        bounded_query(sqlx::query(V2.sql[0]).execute(main.as_ref())).await?;
        let operation = Uuid::new_v4();
        bounded_query(
            sqlx::query("INSERT INTO kuru_migrations VALUES (2, ?, ?, ?)")
                .bind(V2.id)
                .bind(digest(&V2))
                .bind(operation.hyphenated().to_string())
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("UPDATE kuru_schema SET version = 2 WHERE id = 1").execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Create branch-free schema v2 capacity fixture").await?;
        validate_version_with(TEST_REGISTRY, &main, 2).await?;
        upgrade_with(REGISTRY, &server, &main, &MigrationRunnerHooks::none()).await?;
        validate_version_with(TEST_REGISTRY, &main, 5).await?;

        let base = revision(&main).await?;
        let mut names = Vec::with_capacity(INVENTORY_LIMIT);
        let existing = reserved_names(&main).await?.len();
        for _ in existing..INVENTORY_LIMIT {
            let name = attempt_name(6, Uuid::new_v4());
            bounded_query(
                sqlx::query("CALL DOLT_BRANCH(?, ?)")
                    .bind(&name)
                    .bind(&base)
                    .fetch_all(main.as_ref()),
            )
            .await?;
            let attempt = server.pool(&name).await?;
            let prepared = bounded_query(sqlx::query(V6.sql[0]).execute(attempt.as_ref())).await;
            after_cleanup(prepared.map(|_| ()), close_branch_pool(&attempt).await)?;
            names.push(name);
        }
        assert_eq!(reserved_names(&main).await?.len(), INVENTORY_LIMIT);
        let before = durable_snapshot(&main).await?;
        let attempts_before = snapshot_attempts(&server, &names).await?;
        assert_failed_runner_unchanged(
            TEST_REGISTRY,
            &server,
            &main,
            &before,
            "leave no capacity for a fresh attempt",
        )
        .await?;
        assert_attempts_unchanged(&server, attempts_before).await?;
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn migration_attempt_rejection_preserves_invalid_inventory() -> Result<()> {
        #[derive(Clone, Copy)]
        enum InvalidInventory {
            Malformed,
            UnknownTarget,
            Excess,
        }

        for invalid in [
            InvalidInventory::Malformed,
            InvalidInventory::UnknownTarget,
            InvalidInventory::Excess,
        ]
        .into_iter()
        {
            let store = super::super::MemoryStore::temporary().await?;
            let base = store.revision().await?;
            let names = match invalid {
                InvalidInventory::Malformed => vec!["kuru_migration_bad".to_owned()],
                InvalidInventory::UnknownTarget => {
                    vec![attempt_name(7, Uuid::new_v4())]
                }
                InvalidInventory::Excess => {
                    let existing = reserved_names(&store.pool).await?.len();
                    ensure!(
                        existing <= INVENTORY_LIMIT,
                        "fixture inventory is already excessive"
                    );
                    (0..=(INVENTORY_LIMIT - existing))
                        .map(|_| attempt_name(7, Uuid::new_v4()))
                        .collect()
                }
            };
            for name in &names {
                bounded_query(
                    sqlx::query("CALL DOLT_BRANCH(?, ?)")
                        .bind(name)
                        .bind(&base)
                        .fetch_all(store.pool.as_ref()),
                )
                .await?;
            }
            let before = durable_snapshot(&store.pool).await?;
            let attempts_before = snapshot_attempts(&store.shared.server, &names).await?;
            assert_failed_runner_unchanged(
                TEST_REGISTRY,
                &store.shared.server,
                &store.pool,
                &before,
                match invalid {
                    InvalidInventory::Malformed => "reserved migration branch is malformed",
                    InvalidInventory::UnknownTarget => "unsupported Dolt memory schema transition",
                    InvalidInventory::Excess => "too many retained Dolt migration attempts",
                },
            )
            .await?;
            assert_attempts_unchanged(&store.shared.server, attempts_before).await?;
            store.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_v7_receipt_order_and_operation_uniqueness_are_enforced() -> Result<()> {
        for repeated_operation in [false, true] {
            let store = super::super::MemoryStore::temporary().await?;
            upgrade_with(
                TEST_REGISTRY,
                &store.shared.server,
                &store.pool,
                &MigrationRunnerHooks::none(),
            )
            .await?;
            if repeated_operation {
                bounded_query(
                    sqlx::query("ALTER TABLE kuru_migrations DROP INDEX operation")
                        .execute(store.pool.as_ref()),
                )
                .await?;
                let operation: String = bounded_query(
                    sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = 2")
                        .fetch_one(store.pool.as_ref()),
                )
                .await?;
                bounded_query(
                    sqlx::query("UPDATE kuru_migrations SET operation = ? WHERE version = 7")
                        .bind(operation)
                        .execute(store.pool.as_ref()),
                )
                .await?;
            } else {
                bounded_query(
                    sqlx::query("UPDATE kuru_migrations SET version = 1 WHERE version = 2")
                        .execute(store.pool.as_ref()),
                )
                .await?;
            }
            commit_fixture(&store.pool, "Persist invalid v3 receipt chain fixture").await?;
            let before = durable_snapshot(&store.pool).await?;
            assert_failed_runner_unchanged(
                TEST_REGISTRY,
                &store.shared.server,
                &store.pool,
                &before,
                if repeated_operation {
                    "receipt operation is repeated"
                } else {
                    "receipt version is out of order"
                },
            )
            .await?;
            store.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn persisted_v1_future_receipt_authority_fails_before_new_attempt() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "b".repeat(64)),
        )?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        bounded_query(sqlx::query(V2.sql[0]).execute(main.as_ref())).await?;
        bounded_query(
            sqlx::query("INSERT INTO kuru_migrations VALUES (2, ?, ?, ?)")
                .bind(V2.id)
                .bind(digest(&V2))
                .bind(Uuid::new_v4().hyphenated().to_string())
                .execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Persist future receipt authority in schema v1").await?;
        let before = durable_snapshot(&main).await?;
        assert_failed_runner_unchanged(
            REGISTRY,
            &server,
            &main,
            &before,
            "schema 1 must not contain migration receipt authority",
        )
        .await?;
        assert!(
            before
                .refs
                .iter()
                .all(|(name, _)| !name.starts_with(RESERVED_PREFIX)),
            "fixture must prove rejection before a reserved attempt is created"
        );
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn historical_and_out_of_order_attempts_fail_without_mutation() -> Result<()> {
        let store = super::super::MemoryStore::temporary().await?;
        let operation = Uuid::new_v4();
        let name = attempt_name(2, operation);
        let completed_v2 = reserved_names(&store.pool)
            .await?
            .into_iter()
            .find(|name| parse_attempt(name).is_ok_and(|(target, _)| target == 2))
            .context("temporary store did not retain its v2 migration branch")?;
        let completed_v2_pool = store.shared.server.pool(&completed_v2).await?;
        let head = revision(&completed_v2_pool).await?;
        completed_v2_pool.close().await;
        drop(completed_v2_pool);
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&name)
                .bind(&head)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        let before = durable_snapshot(&store.pool).await?;
        let attempt = store.shared.server.pool(&name).await?;
        let attempt_before = durable_snapshot(&attempt).await?;
        attempt.close().await;
        drop(attempt);
        assert_failed_runner_unchanged(
            TEST_REGISTRY,
            &store.shared.server,
            &store.pool,
            &before,
            "receipt does not match its branch",
        )
        .await?;
        let attempt = store.shared.server.pool(&name).await?;
        assert_eq!(durable_snapshot(&attempt).await?, attempt_before);
        attempt.close().await;
        drop(attempt);
        store.close().await?;

        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "a".repeat(64)),
        )?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        let v1_base = revision(&main).await?;
        let divergent = format!("candidate_{}", Uuid::new_v4().simple());
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&divergent)
                .bind(&v1_base)
                .fetch_all(main.as_ref()),
        )
        .await?;
        let divergent_pool = server.pool(&divergent).await?;
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                .bind(b"fixture".as_slice())
                .bind(b"user".as_slice())
                .bind("divergent v1 history")
                .execute(divergent_pool.as_ref()),
        )
        .await?;
        commit_fixture(&divergent_pool, "Create divergent v1 fixture").await?;
        let divergent_head = revision(&divergent_pool).await?;
        divergent_pool.close().await;
        drop(divergent_pool);
        upgrade_with(REGISTRY, &server, &main, &MigrationRunnerHooks::none()).await?;
        let operation = Uuid::new_v4();
        let non_ancestral_name = attempt_name(2, operation);
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&non_ancestral_name)
                .bind(&divergent_head)
                .fetch_all(main.as_ref()),
        )
        .await?;
        let non_ancestral = server.pool(&non_ancestral_name).await?;
        build_attempt(
            REGISTRY,
            &non_ancestral,
            &V2,
            operation,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        validate_attempt(REGISTRY, &non_ancestral, &V2, operation, &divergent_head).await?;
        let non_ancestral_before = durable_snapshot(&non_ancestral).await?;
        non_ancestral.close().await;
        drop(non_ancestral);
        let before = durable_snapshot(&main).await?;
        assert_failed_runner_unchanged(
            REGISTRY,
            &server,
            &main,
            &before,
            "not retained by active history",
        )
        .await?;
        let non_ancestral = server.pool(&non_ancestral_name).await?;
        assert_eq!(
            durable_snapshot(&non_ancestral).await?,
            non_ancestral_before
        );
        non_ancestral.close().await;
        drop(non_ancestral);
        main.close().await;
        drop(main);
        server.close().await?;

        let store = super::super::MemoryStore::temporary().await?;
        let completed_name = reserved_names(&store.pool)
            .await?
            .into_iter()
            .find(|name| parse_attempt(name).is_ok_and(|(target, _)| target == CURRENT_VERSION))
            .context("temporary store did not retain its current migration branch")?;
        let completed = store.shared.server.pool(&completed_name).await?;
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                .bind(b"fixture".as_slice())
                .bind(b"assistant".as_slice())
                .bind("advanced retained migration branch")
                .execute(completed.as_ref()),
        )
        .await?;
        commit_fixture(&completed, "Advance retained migration branch").await?;
        completed.close().await;
        drop(completed);
        bounded_query(
            sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                .bind(&completed_name)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        let completed = store.shared.server.pool(&completed_name).await?;
        let completed_before = durable_snapshot(&completed).await?;
        completed.close().await;
        drop(completed);
        let before = durable_snapshot(&store.pool).await?;
        assert_failed_runner_unchanged(
            TEST_REGISTRY,
            &store.shared.server,
            &store.pool,
            &before,
            "schema version 5, expected 4",
        )
        .await?;
        let completed = store.shared.server.pool(&completed_name).await?;
        assert_eq!(durable_snapshot(&completed).await?, completed_before);
        completed.close().await;
        drop(completed);
        store.close().await?;

        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "f".repeat(64)),
        )?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        let base = revision(&main).await?;
        let name = attempt_name(5, Uuid::new_v4());
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&name)
                .bind(&base)
                .fetch_all(main.as_ref()),
        )
        .await?;
        let before = durable_snapshot(&main).await?;
        assert_failed_runner_unchanged(TEST_REGISTRY, &server, &main, &before, "out-of-order step")
            .await?;
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn retained_v2_attempts_and_candidate_survive_test_v7_progression() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "3".repeat(64)),
        )?;
        let store = super::super::MemoryStore::open(options.clone()).await?;
        store
            .append("conversation", "user", "written after the v2 upgrade")
            .await?;
        let candidate = store.begin_candidate("pre-v7 candidate").await?;
        candidate
            .view()
            .append("candidate", "assistant", "kept on schema v6")
            .await?;
        let candidate_head = candidate.view().revision().await?;

        let completed_name = reserved_names(&store.pool)
            .await?
            .into_iter()
            .find(|name| parse_attempt(name).is_ok_and(|(target, _)| target == 2))
            .context("temporary store did not retain its v2 migration branch")?;
        let completed = store.shared.server.pool(&completed_name).await?;
        let completed_head = revision(&completed).await?;
        let base: String = bounded_query(
            sqlx::query_scalar(
                "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
            )
            .bind(&completed_head)
            .fetch_one(completed.as_ref()),
        )
        .await?;
        completed.close().await;
        drop(completed);

        let failed_operation = Uuid::new_v4();
        let failed_name = attempt_name(2, failed_operation);
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&failed_name)
                .bind(&base)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        let failed = store.shared.server.pool(&failed_name).await?;
        bounded_query(sqlx::query(V2.sql[0]).execute(failed.as_ref())).await?;
        assert!(retained_failed_shape(&failed, &V2).await?);
        let failed_head = revision(&failed).await?;
        failed.close().await;
        drop(failed);

        assert_eq!(
            validate_ready_with(TEST_REGISTRY, &store.shared.server, &store.pool).await?,
            6
        );
        let v7_operation = Uuid::new_v4();
        let v7_name = attempt_name(7, v7_operation);
        let current_base = store.revision().await?;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&v7_name)
                .bind(&current_base)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert!(
            validate_ready_with(TEST_REGISTRY, &store.shared.server, &store.pool)
                .await
                .is_err(),
            "a ready v7 stage must reject while an earlier failed attempt remains"
        );

        upgrade_with(
            TEST_REGISTRY,
            &store.shared.server,
            &store.pool,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        validate_active_with(TEST_REGISTRY, &store.shared.server, &store.pool).await?;
        assert_eq!(version(&store.pool).await?, 7);
        let v7_attempt = store.shared.server.pool(&v7_name).await?;
        assert_eq!(revision(&v7_attempt).await?, store.revision().await?);
        let v7_receipt: String = bounded_query(
            sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = 7")
                .fetch_one(v7_attempt.as_ref()),
        )
        .await?;
        assert_eq!(v7_receipt, v7_operation.hyphenated().to_string());
        v7_attempt.close().await;
        drop(v7_attempt);
        assert_eq!(
            reserved_names(&store.pool)
                .await?
                .into_iter()
                .filter(|name| parse_attempt(name).is_ok_and(|(target, _)| target == 7))
                .count(),
            1,
            "the pristine exact-base v7 attempt must be reused"
        );
        // This is a synthetic future schema, beyond the current store API's
        // validated open contract. Inspect the test-registry-backed SQL view.
        let retained_message: String = bounded_query(
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"conversation".as_slice())
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(retained_message, "written after the v2 upgrade");

        let completed = store.shared.server.pool(&completed_name).await?;
        assert_eq!(revision(&completed).await?, completed_head);
        validate_version_with(TEST_REGISTRY, &completed, 2).await?;
        completed.close().await;
        drop(completed);
        let failed = store.shared.server.pool(&failed_name).await?;
        assert_eq!(revision(&failed).await?, failed_head);
        assert!(retained_failed_shape(&failed, &V2).await?);
        failed.close().await;
        drop(failed);

        let old_view = candidate.view();
        let old_candidate_name = old_view.branch.clone();
        assert_eq!(old_view.revision().await?, candidate_head);
        assert_eq!(
            validate_supported_with(TEST_REGISTRY, &old_view.pool).await?,
            5
        );
        assert_eq!(
            old_view.history("candidate", 10).await?[0].plain_text(),
            Some("kept on schema v6")
        );
        let before_stale_merge = durable_snapshot(&store.pool).await?;
        assert!(
            bounded_query(
                sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                    .bind(&old_candidate_name)
                    .fetch_all(store.pool.as_ref()),
            )
            .await
            .is_err(),
            "pre-v6 candidate unexpectedly fast-forwarded into v6 main"
        );
        assert_eq!(durable_snapshot(&store.pool).await?, before_stale_merge);

        // Model a future v6 writer through its test registry and SQL view.
        // Released-v5 reopen refusal is checked in the separate old-registry fixture.
        let fresh_name = format!("candidate_{}", Uuid::new_v4().simple());
        let fresh_base = revision(&store.pool).await?;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&fresh_name)
                .bind(&fresh_base)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        let fresh = store.shared.server.pool(&fresh_name).await?;
        assert_eq!(validate_supported_with(TEST_REGISTRY, &fresh).await?, 7);
        bounded_query(
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(b"post-v6".as_slice())
                .bind(json!({"preserved": true}).to_string())
                .execute(fresh.as_ref()),
        )
        .await?;
        commit_fixture(&fresh, "Test future-schema candidate write").await?;
        let fresh_head = revision(&fresh).await?;
        fresh.close().await;
        drop(fresh);
        bounded_query(
            sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                .bind(&fresh_name)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(revision(&store.pool).await?, fresh_head);
        bounded_query(
            sqlx::query(
                "INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, ?, ?)",
            )
            .bind(b"conversation".as_slice())
            .bind(b"assistant".as_slice())
            .bind("text-v1")
            .bind("written after schema v6")
            .execute(store.pool.as_ref()),
        )
        .await?;
        commit_fixture(&store.pool, "Test future-schema conversation write").await?;
        validate_active_with(TEST_REGISTRY, &store.shared.server, &store.pool).await?;
        assert_eq!(version(&store.pool).await?, 7);
        let promoted_value: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(b"post-v6".as_slice())
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&promoted_value)?,
            json!({"preserved": true})
        );

        let final_head = store.revision().await?;
        let final_refs: BTreeSet<(String, String)> = {
            let mut refs = BTreeSet::new();
            for name in reserved_names(&store.pool).await? {
                let hash: String = bounded_query(
                    sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE name = ?")
                        .bind(&name)
                        .fetch_one(store.pool.as_ref()),
                )
                .await?;
                refs.insert((name, hash));
            }
            refs
        };
        let old_pool = old_view.pool.clone();
        drop(old_view);
        drop(candidate);
        tokio::time::timeout(QUERY_TIMEOUT, old_pool.close())
            .await
            .context("close old candidate fixture pool")?;
        store.close().await?;

        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        upgrade_with(TEST_REGISTRY, &server, &main, &MigrationRunnerHooks::none()).await?;
        validate_active_with(TEST_REGISTRY, &server, &main).await?;
        assert_eq!(revision(&main).await?, final_head);
        let reopened_refs: BTreeSet<(String, String)> = {
            let mut refs = BTreeSet::new();
            for name in reserved_names(&main).await? {
                let hash: String = bounded_query(
                    sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE name = ?")
                        .bind(&name)
                        .fetch_one(main.as_ref()),
                )
                .await?;
                refs.insert((name, hash));
            }
            refs
        };
        assert_eq!(reopened_refs, final_refs);
        let messages: Vec<String> = bounded_query(
            sqlx::query_scalar(
                "SELECT content FROM messages WHERE namespace = ? ORDER BY sequence",
            )
            .bind(b"conversation".as_slice())
            .fetch_all(main.as_ref()),
        )
        .await?;
        assert_eq!(
            messages,
            ["written after the v2 upgrade", "written after schema v6"]
        );
        let value: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(b"post-v6".as_slice())
                .fetch_one(main.as_ref()),
        )
        .await?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&value)?,
            json!({"preserved": true})
        );
        let v6_commits: i64 = bounded_query(
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 6 [%]'",
            )
            .fetch_one(main.as_ref()),
        )
        .await?;
        assert_eq!(v6_commits, 1, "cold reopen must not replay schema v6");
        let failed = server.pool(&failed_name).await?;
        assert_eq!(revision(&failed).await?, failed_head);
        assert!(retained_failed_shape(&failed, &V2).await?);
        failed.close().await;
        drop(failed);
        let old_candidate = server.pool(&old_candidate_name).await?;
        assert_eq!(revision(&old_candidate).await?, candidate_head);
        assert_eq!(
            validate_supported_with(TEST_REGISTRY, &old_candidate).await?,
            5
        );
        let old_content: String = bounded_query(
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"candidate".as_slice())
                .fetch_one(old_candidate.as_ref()),
        )
        .await?;
        assert_eq!(old_content, "kept on schema v5");
        old_candidate.close().await;
        drop(old_candidate);
        main.close().await;
        drop(main);
        server.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn current_upgrade_migrates_usage_without_rewriting_historical_candidate() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "a".repeat(64)),
        )?;
        released_v2(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        bounded_query(sqlx::query(V3.sql[0]).execute(main.as_ref())).await?;
        bounded_query(
            sqlx::query("INSERT INTO kuru_migrations VALUES (3, ?, ?, ?)")
                .bind(V3.id)
                .bind(digest(&V3))
                .bind(Uuid::new_v4().hyphenated().to_string())
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("UPDATE kuru_schema SET version = 3 WHERE id = 1").execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Release schema v3 with permanent usage fixture").await?;
        let base = revision(&main).await?;
        let candidate = format!("candidate_{}", Uuid::new_v4().simple());
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&candidate)
                .bind(&base)
                .fetch_all(main.as_ref()),
        )
        .await?;
        let candidate_pool = server.pool(&candidate).await?;
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, ?, ?)")
                .bind(b"private/dream".as_slice())
                .bind(b"assistant".as_slice())
                .bind("text-v1")
                .bind("historical candidate")
                .execute(candidate_pool.as_ref()),
        )
        .await?;
        commit_fixture(
            &candidate_pool,
            "Historical candidate before receipt upgrade",
        )
        .await?;
        let old_candidate_head = revision(&candidate_pool).await?;
        candidate_pool.close().await;

        let usage_name = super::super::usage_ledger::BRANCH;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(usage_name)
                .bind(&base)
                .fetch_all(main.as_ref()),
        )
        .await?;
        let usage = server.pool(usage_name).await?;
        let old_receipt = Uuid::new_v4().hyphenated().to_string();
        let prior_key = format!(
            "kuru.usage.v1/session/{}",
            Sha256::digest(b"prior")
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        bounded_query(
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(prior_key.as_bytes())
                .bind(r#"{"format":1,"session_id":"prior","historical_complete":false}"#)
                .execute(usage.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
                .bind(&old_receipt)
                .bind("historical usage marker")
                .execute(usage.as_ref()),
        )
        .await?;
        commit_fixture(&usage, "Historical usage before receipt upgrade").await?;
        let old_usage_head = revision(&usage).await?;
        usage.close().await;
        main.close().await;
        server.close().await?;

        let store = super::super::MemoryStore::open(options).await?;
        assert_eq!(version(&store.pool).await?, CURRENT_VERSION);
        let usage = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("upgraded usage pool missing")?;
        assert_eq!(version(&usage).await?, 4);
        assert_ne!(revision(&usage).await?, old_usage_head);
        let legacy_format: i32 = bounded_query(
            sqlx::query_scalar("SELECT receipt_format FROM operations WHERE id = ?")
                .bind(&old_receipt)
                .fetch_one(usage.as_ref()),
        )
        .await?;
        assert_eq!(
            legacy_format, 0,
            "old receipt gained a fabricated request identity"
        );
        let prior: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(prior_key.as_bytes())
                .fetch_one(usage.as_ref()),
        )
        .await?;
        assert!(prior.contains("\"session_id\":\"prior\""));
        store.usage_ledger()?.mark_new_session("new").await?;
        let receipt_count: i64 = bounded_query(
            sqlx::query_scalar("SELECT COUNT(*) FROM operations").fetch_one(usage.as_ref()),
        )
        .await?;
        assert_eq!(receipt_count, 2, "new usage write erased legacy receipt");

        let old_candidate = store.shared.server.pool(&candidate).await?;
        assert_eq!(revision(&old_candidate).await?, old_candidate_head);
        assert_eq!(version(&old_candidate).await?, 3);
        let old_content: String = bounded_query(
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"private/dream".as_slice())
                .fetch_one(old_candidate.as_ref()),
        )
        .await?;
        assert_eq!(old_content, "historical candidate");
        old_candidate.close().await;
        let fresh = store.begin_candidate("after receipt upgrade").await?;
        assert_eq!(version(&fresh.view().pool).await?, CURRENT_VERSION);
        drop(fresh);
        drop(usage);
        store.close().await
    }

    #[tokio::test]
    async fn released_v5_summary_identity_survives_v6_upgrade_reopen_and_export() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "6".repeat(64)),
        )?;
        released_v2(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        upgrade_with(
            RELEASED_V5_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        assert_eq!(version(&main).await?, 5);
        let inserted = bounded_query(
            sqlx::query("INSERT INTO messages (namespace, session_id, role, content_format, content) VALUES (?, ?, ?, 'text-v1', ?)")
                .bind(b"actor namespace".as_slice())
                .bind(b"session".as_slice())
                .bind(b"user".as_slice())
                .bind("legacy compact source")
                .execute(main.as_ref()),
        )
        .await?;
        let through_sequence = i64::try_from(inserted.last_insert_id())
            .context("legacy compact source sequence exceeds integer range")?;
        commit_fixture(&main, "Released v5 compact source").await?;
        let source_revision = revision(&main).await?;
        let context = super::super::ContextSummaryRecord {
            actor_namespace: "actor namespace".into(),
            session_id: "session".into(),
            source_namespace: "actor namespace".into(),
            summary_namespace: "summary namespace".into(),
            source_view: "main".into(),
            source_revision: source_revision.clone(),
            after_sequence: 0,
            through_sequence,
            turn_id: Some("turn".into()),
            operation_id: None,
            producer_actor_id: None,
            invocation_id: "invocation".into(),
            summary: "released v5 context".into(),
        };
        let summary_id = super::super::context_summary_id(&context)?;
        let reasoning = super::super::ReasoningSummaryRecord {
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            operation_id: None,
            actor_id: "actor".into(),
            invocation_id: "invocation".into(),
            item_id: Some("item".into()),
            output_index: Some(0),
            summary_index: 0,
            text: "released v5 private reasoning".into(),
        };
        let reasoning_key = super::super::reasoning_summary_key(&reasoning)?;
        let reasoning_json = serde_json::to_string(&reasoning)?;
        bounded_query(
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(reasoning_key.as_bytes())
                .bind(&reasoning_json)
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("INSERT INTO context_summaries (summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, invocation_id, record_format, summary) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&summary_id)
                .bind(context.actor_namespace.as_bytes())
                .bind(context.session_id.as_bytes())
                .bind(context.source_namespace.as_bytes())
                .bind(context.summary_namespace.as_bytes())
                .bind(&context.source_view)
                .bind(&context.source_revision)
                .bind(context.after_sequence)
                .bind(context.through_sequence)
                .bind(context.turn_id.as_deref().context("legacy turn missing")?.as_bytes())
                .bind(context.invocation_id.as_bytes())
                .bind(super::super::CONTEXT_SUMMARY_FORMAT)
                .bind(&context.summary)
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("INSERT INTO context_summary_cursors (actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(context.actor_namespace.as_bytes())
                .bind(context.session_id.as_bytes())
                .bind(context.source_namespace.as_bytes())
                .bind(context.through_sequence)
                .bind(&summary_id)
                .bind(&context.source_view)
                .bind(&context.source_revision)
                .execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Released v5 summary fixture").await?;
        main.close().await;
        drop(main);
        server.close().await?;

        let store = super::super::MemoryStore::open(options.clone()).await?;
        assert_eq!(version(&store.pool).await?, CURRENT_VERSION);
        type MigratedSummaryRow = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>, String);
        let row: MigratedSummaryRow = bounded_query(
            sqlx::query_as("SELECT turn_id, operation_id, producer_actor_id, record_format FROM context_summaries WHERE summary_id = ?")
                .bind(&summary_id)
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(row.0, Some(b"turn".to_vec()));
        assert_eq!(row.1, None);
        assert_eq!(row.2, None);
        assert_eq!(row.3, super::super::CONTEXT_SUMMARY_FORMAT);
        assert_eq!(
            store.get(&reasoning_key).await?,
            Some(serde_json::from_str(&reasoning_json)?)
        );
        let usage = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage pool missing after v6 upgrade")?;
        assert_eq!(version(&usage).await?, USAGE_CURRENT_VERSION);
        drop(usage);

        let export = store.begin_active_export().await?;
        let mut cursor = None;
        let mut records = Vec::new();
        let reasoning_value: serde_json::Value = serde_json::from_str(&reasoning_json)?;
        loop {
            let page = export.page(cursor).await?;
            records.extend(page.records);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert!(records.iter().any(|record| matches!(record,
            super::super::StorageRecord::ContextSummary { summary_id: exported_id, record, record_format }
                if exported_id == &summary_id
                    && record == &context
                    && record_format == super::super::CONTEXT_SUMMARY_FORMAT
        )));
        assert!(records.iter().any(|record| matches!(record,
            super::super::StorageRecord::State { key, value }
                if key == &reasoning_key && value == &reasoning_value
        )));
        let upgraded_head = store.revision().await?;
        store.close().await?;

        let reopened = super::super::MemoryStore::open(options).await?;
        assert_eq!(reopened.revision().await?, upgraded_head);
        let window = reopened
            .context_summary_window(
                &context.actor_namespace,
                &context.summary_namespace,
                Some(&context.session_id),
                Some(&context.source_namespace),
                16,
            )
            .await?;
        assert_eq!(window.records.len(), 1);
        assert_eq!(window.records[0].summary_id, summary_id);
        assert_eq!(window.records[0].record, context);
        reopened.close().await
    }
}
