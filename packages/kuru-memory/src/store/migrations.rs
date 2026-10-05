//! Private, immutable Dolt schema migration registry.
//!
//! A migration changes a versioned database, never an activation record or the
//! sidecar identity protocol.  Keeping the definitions here makes the receipt
//! validator the authority for both cold discovery and ordinary startup.
use super::{
    AUTHOR, LEGACY_PREFIX_RECORD_FORMAT, LegacyTranscriptPrefix, QUERY_TIMEOUT,
    SESSION_CATALOG_RECORD_FORMAT, SessionCatalogRecord, SessionLifecycleState, revision,
    validate_schema_v1, validate_session_catalog,
};
use crate::pool::{MemoryPool, PooledSession};
use anyhow::{Context, Result, bail, ensure};
use kuru_core::Mode;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection, Executor, MySql, MySqlConnection, Row};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::server::Server;

pub(super) const CURRENT_VERSION: i32 = 10;
pub(super) const STATE_VERSION: i32 = 9;
pub(super) const USAGE_CURRENT_VERSION: i32 = 4;
/// The main schema step that introduces publication records: every main step
/// at or above it records its own publication inside its attempt commit.
const PUBLICATION_VERSION: i32 = 8;
/// The format of a `kuru_migration_publications` row. The store template key
/// covers it, so a format change alone also rebuilds the template.
pub(super) const PUBLICATION_RECORD_FORMAT: i8 = 1;
/// The engine database every branch and revision database name qualifies.
const DATABASE: &str = "kuru";
const RESERVED_PREFIX: &str = "kuru_migration_";
const USAGE_RESERVED_PREFIX: &str = "kuru_usage_migration_";
const INVENTORY_LIMIT: usize = 64;
const DEFINITION_LIMIT: usize = 64;
const FIELD_LIMIT: usize = 1024;
const MIGRATION_ID_LIMIT: usize = 128;
const RECEIPT_PROTOCOL: &str = "kuru.memory.migration.receipt.v1";
/// The statements and names every migration step runs besides its own
/// definition: they shape each store's history, so the store template key
/// covers them ([`TEMPLATE_KEY_STATEMENTS`]); call sites use these constants.
const BRANCH_CREATE: &str = "CALL DOLT_BRANCH(?, ?)";
const ATTEMPT_BEGIN: &str = "START TRANSACTION";
const ATTEMPT_ADVANCE: &str = "UPDATE kuru_schema SET version = ? WHERE id = 1 AND version = ?";
const ATTEMPT_RECEIPT: &str =
    "INSERT INTO kuru_migrations (version, id, digest, operation) VALUES (?, ?, ?, ?)";
/// One publication record, written in the attempt's own transaction after its
/// receipt so it reaches main only with the step's exact-base fast-forward.
const ATTEMPT_RECORD: &str = "INSERT INTO kuru_migration_publications (version, branch, base, operation, definition_digest, record_format) VALUES (?, ?, ?, ?, ?, ?)";
const ATTEMPT_COMMIT: &str = "CALL DOLT_COMMIT('-Am', ?, '--author', ?)";
/// The attempt commit message around its target version and operation.
const ATTEMPT_MESSAGE: [&str; 3] = ["Upgrade Kuru memory schema ", " [", "]"];
const ATTEMPT_END: &str = "COMMIT";
const PUBLISH_MERGE: &str = "CALL DOLT_MERGE(?, '--ff-only')";
/// Every migration-path statement and name the store template key covers.
pub(super) const TEMPLATE_KEY_STATEMENTS: [&str; 15] = [
    BRANCH_CREATE,
    ATTEMPT_BEGIN,
    ATTEMPT_ADVANCE,
    ATTEMPT_RECEIPT,
    ATTEMPT_RECORD,
    ATTEMPT_COMMIT,
    ATTEMPT_MESSAGE[0],
    ATTEMPT_MESSAGE[1],
    ATTEMPT_MESSAGE[2],
    ATTEMPT_END,
    PUBLISH_MERGE,
    RESERVED_PREFIX,
    USAGE_RESERVED_PREFIX,
    RECEIPT_PROTOCOL,
    super::usage_ledger::BRANCH,
];

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
    route_source: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<MemoryPool>>>>,
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
    route_source: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<MemoryPool>>>>,
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
    pub(super) async fn route_source(&self) -> Result<std::sync::Arc<MemoryPool>> {
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
    direct: &MemoryPool,
    hooks: &MigrationRunnerHooks,
    boundary: MigrationBoundary,
) -> Result<Option<MemoryPool>> {
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
        Ok(Some(MemoryPool::fixture(
            sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(QUERY_TIMEOUT)
                .connect_with(options)
                .await
                .context("connect migration fixture proxy")?,
            "main",
        )))
    }
}

/// A migration command connection's budget: its acquisition and identity
/// statement share one `QUERY_TIMEOUT`. Each migration statement keeps its
/// own [`bounded_query`] budget, so a fixture pause between them is not
/// charged to the connection.
fn connection_deadline() -> tokio::time::Instant {
    super::write_deadline()
}

async fn bounded_query<T>(
    query: impl std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
) -> Result<T> {
    crate::pool::within(QUERY_TIMEOUT, query)
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct LegacySession {
    id: String,
    mode: Mode,
    turns: usize,
    label: String,
    #[serde(default)]
    last_completed_speaker: Option<String>,
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

const V7: Definition = Definition {
    from: 6,
    to: 7,
    id: "kuru.memory.session-lifecycle.v7",
    sql: &[
        "CREATE TABLE session_catalog (session_id VARBINARY(128) PRIMARY KEY, mode VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, label VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL, created_order BIGINT NOT NULL, updated_order BIGINT NOT NULL, lifecycle_generation BIGINT NOT NULL, lifecycle_state VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, head_node_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL, pending_node_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL, legacy_prefix LONGTEXT CHARACTER SET utf8mb4 NULL, fork_provenance LONGTEXT CHARACTER SET utf8mb4 NULL, record_format VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, UNIQUE INDEX session_catalog_created_order (created_order, session_id), INDEX session_catalog_lifecycle_order (lifecycle_state, updated_order, session_id))",
        "CREATE TABLE session_public_turns (node_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY, origin_session_id VARBINARY(128) NOT NULL, turn_id VARBINARY(128) NOT NULL, record_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, continuation_of_node_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL, predecessor_node_id CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL, settlement VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, user_entry LONGTEXT CHARACTER SET utf8mb4 NULL, speaker_id VARBINARY(1024) NULL, terminal_entries LONGTEXT CHARACTER SET utf8mb4 NULL, record_format VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, UNIQUE INDEX session_public_turn_identity (origin_session_id, turn_id, record_kind), INDEX session_public_turn_predecessor (predecessor_node_id), INDEX session_public_turn_continuation (continuation_of_node_id))",
    ],
    transform: "validated legacy session metadata may acquire catalog and immutable prefix descriptors; source state, messages and v6 provenance remain unchanged",
    postcondition: "version=7;typed session catalog and immutable public-turn chain;legacy bytes and v6 compact provenance retained;v4 usage registry retained",
    failed_status: &[
        StatusRow {
            table: "session_catalog",
            staged: 0,
            status: "new table",
        },
        StatusRow {
            table: "session_public_turns",
            staged: 0,
            status: "new table",
        },
    ],
};

const V8: Definition = Definition {
    from: 7,
    to: 8,
    id: "kuru.memory.migration-publications.v8",
    sql: &[
        "CREATE TABLE kuru_migration_publications (version INT PRIMARY KEY, branch VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE, base CHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, operation CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, definition_digest CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL, record_format TINYINT NOT NULL)",
    ],
    transform: "published main branches this open fully classified acquire publication records; no record is inferred from a branch name",
    postcondition: "version=8;one publication record per published main step;v7 session lifecycle and v4 usage registry retained",
    failed_status: &[StatusRow {
        table: "kuru_migration_publications",
        staged: 0,
        status: "new table",
    }],
};

const V9: Definition = Definition {
    from: 8,
    to: 9,
    id: "kuru.memory.state-versions.v9",
    sql: &["ALTER TABLE state ADD COLUMN version BIGINT NOT NULL DEFAULT 0"],
    transform: "existing state values remain byte-for-byte unchanged and acquire version zero; topology remains owned by legacy writers until split publication lands",
    postcondition: "version=9;state.version is signed BIGINT NOT NULL DEFAULT 0;v8 publication records remain exact",
    failed_status: &[StatusRow {
        table: "state",
        staged: 0,
        status: "modified",
    }],
};

mod topology_state;

const V10: Definition = Definition {
    from: 9,
    to: 10,
    id: "kuru.memory.split-topology.v10",
    sql: &[],
    transform: "then-latest mode topology materializes membership.v1 and SHA256 exact-identity state_report.v1 rows; legacy bytes stay unchanged; project focus is not assigned to sessions",
    postcondition: "version=10;all legacy mode topology destinations materialized atomically with conflict refusal;new rows have version zero;existing equal destinations retain their versions",
    // This step has no nontransactional DDL. A refused transform rolls back to
    // its pristine source; dirty state is never a permitted failed attempt.
    failed_status: &[],
};

const DEFINITIONS: &[Definition] = &[V2, V3, V4, V5, V6, V7, V8, V9, V10];

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
                (!definition.sql.is_empty() || definition.id == V10.id)
                    && definition.sql.len() <= DEFINITION_LIMIT,
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

/// The receipt digest of every definition of the main and the usage
/// registries, in declared order: the migration chain as the store template
/// key covers it.
pub(super) fn template_key_definitions() -> [Vec<String>; 2] {
    [REGISTRY, USAGE_REGISTRY].map(|registry| registry.definitions.iter().map(digest).collect())
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

const VERSION_QUERY: &str = "SELECT id, version FROM kuru_schema LIMIT 2";

pub(super) async fn version(pool: &MemoryPool) -> Result<i32> {
    version_from(pool).await
}

async fn version_on(connection: &mut MySqlConnection) -> Result<i32> {
    version_from(connection).await
}

async fn version_from<'e>(executor: impl sqlx::Executor<'e, Database = MySql>) -> Result<i32> {
    let rows = bounded_query(sqlx::query(VERSION_QUERY).fetch_all(executor))
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

pub(super) async fn validate_supported(pool: &MemoryPool) -> Result<i32> {
    validate_supported_with(REGISTRY, pool).await
}

/// Validate a historical branch through its own version dispatcher.
/// This never migrates the branch or applies latest-main validation.
pub(super) async fn validate_historical(pool: &MemoryPool) -> Result<i32> {
    let version = validate_supported_with(REGISTRY, pool).await?;
    authority_working_set(pool).await?;
    Ok(version)
}

async fn validate_supported_with(registry: Registry, pool: &MemoryPool) -> Result<i32> {
    registry.validate()?;
    let mut connection = acquire(pool).await?;
    let found = version_on(&mut connection).await?;
    validate_version_on(registry, &mut connection, found).await?;
    connection.release().await;
    Ok(found)
}

/// One pooled connection for a whole validation pass. The schema validator
/// runs on one session so the same checks can run on a detached connection
/// whose database was switched to a revision (see `RevisionReader`).
async fn acquire(pool: &MemoryPool) -> Result<PooledSession> {
    crate::pool::within(QUERY_TIMEOUT, pool.acquire())
        .await
        .context("Dolt migration query deadline exceeded")?
}

async fn validate_version_with(registry: Registry, pool: &MemoryPool, expected: i32) -> Result<()> {
    let mut connection = acquire(pool).await?;
    validate_version_on(registry, &mut connection, expected).await?;
    connection.release().await;
    Ok(())
}

async fn validate_version_on(
    registry: Registry,
    connection: &mut MySqlConnection,
    expected: i32,
) -> Result<()> {
    let found = version_on(connection).await?;
    ensure!(
        found == expected,
        "Dolt migration branch has schema version {found}, expected {expected}"
    );
    validate_schema_on(registry, connection, expected).await
}

async fn validate_schema_on(
    registry: Registry,
    connection: &mut MySqlConnection,
    found: i32,
) -> Result<()> {
    ensure!(
        (1..=registry.current).contains(&found),
        "unsupported Dolt memory schema version {found}"
    );
    validate_schema_v1(connection).await?;
    if found == 1 {
        let receipt_tables: i64 = bounded_query(
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'kuru_migrations'",
            )
            .fetch_one(&mut *connection),
        )
        .await?;
        ensure!(
            receipt_tables == 0,
            "Dolt memory schema 1 must not contain migration receipt authority"
        );
    }
    if found >= 2 {
        validate_receipts(registry, connection, found).await?;
    }
    if found >= 3 {
        bounded_query(
            sqlx::query("SELECT content_format FROM messages LIMIT 0").fetch_all(&mut *connection),
        )
        .await?;
        let columns = bounded_query(
            sqlx::query("SELECT data_type, is_nullable, column_default, character_maximum_length, character_set_name, collation_name FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'messages' AND BINARY column_name = BINARY 'content_format'")
                .fetch_all(&mut *connection),
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
        validate_operation_receipt_shape(connection).await?;
    }
    if found >= 5 {
        validate_session_provenance_shape(connection, found).await?;
    }
    if found >= 7 {
        validate_session_lifecycle_shape(connection).await?;
    }
    if found >= PUBLICATION_VERSION {
        validate_publication_shape(connection).await?;
    }
    if found >= STATE_VERSION {
        let row = bounded_query(sqlx::query("SELECT data_type, column_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'state' AND BINARY column_name = BINARY 'version'").fetch_one(&mut *connection)).await?;
        ensure!(
            row.try_get::<String, _>(0)?.eq_ignore_ascii_case("bigint")
                && !row
                    .try_get::<String, _>(1)?
                    .to_ascii_lowercase()
                    .contains("unsigned")
                && row.try_get::<String, _>(2)? == "NO"
                && row.try_get::<String, _>(3)?.trim_matches('\'') == "0",
            "state version column differs from schema v9"
        );
    }
    #[cfg(test)]
    if registry.current >= 11 && found >= 11 {
        bounded_query(
            sqlx::query("SELECT marker FROM kuru_migration_test_v11 LIMIT 0")
                .fetch_all(&mut *connection),
        )
        .await?;
    }
    Ok(())
}

async fn validate_publication_shape(connection: &mut MySqlConnection) -> Result<()> {
    bounded_query(
        sqlx::query("SELECT version, branch, base, operation, definition_digest, record_format FROM kuru_migration_publications LIMIT 0")
            .fetch_all(&mut *connection),
    )
    .await?;
    validate_columns(
        connection,
        "kuru_migration_publications",
        &[
            ("version", "int", None, false),
            ("branch", "varchar", Some(128), false),
            ("base", "char", Some(32), false),
            ("operation", "char", Some(36), false),
            ("definition_digest", "char", Some(64), false),
            ("record_format", "tinyint", None, false),
        ],
    )
    .await?;
    validate_index(
        connection,
        "kuru_migration_publications",
        "PRIMARY",
        true,
        &["version"],
    )
    .await?;
    validate_index(
        connection,
        "kuru_migration_publications",
        "branch",
        true,
        &["branch"],
    )
    .await
}

async fn validate_session_lifecycle_shape(connection: &mut MySqlConnection) -> Result<()> {
    bounded_query(
        sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog LIMIT 0")
            .fetch_all(&mut *connection),
    )
    .await?;
    bounded_query(
        sqlx::query("SELECT node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format FROM session_public_turns LIMIT 0")
            .fetch_all(&mut *connection),
    )
    .await?;
    validate_columns(
        connection,
        "session_catalog",
        &[
            ("session_id", "varbinary", Some(128), false),
            ("mode", "varchar", Some(16), false),
            ("label", "varchar", Some(1024), false),
            ("created_order", "bigint", None, false),
            ("updated_order", "bigint", None, false),
            ("lifecycle_generation", "bigint", None, false),
            ("lifecycle_state", "varchar", Some(16), false),
            ("head_node_id", "char", Some(64), true),
            ("pending_node_id", "char", Some(64), true),
            ("legacy_prefix", "longtext", None, true),
            ("fork_provenance", "longtext", None, true),
            ("record_format", "varchar", Some(32), false),
        ],
    )
    .await?;
    validate_columns(
        connection,
        "session_public_turns",
        &[
            ("node_id", "char", Some(64), false),
            ("origin_session_id", "varbinary", Some(128), false),
            ("turn_id", "varbinary", Some(128), false),
            ("record_kind", "varchar", Some(16), false),
            ("continuation_of_node_id", "char", Some(64), true),
            ("predecessor_node_id", "char", Some(64), true),
            ("settlement", "varchar", Some(16), false),
            ("user_entry", "longtext", None, true),
            ("speaker_id", "varbinary", Some(1024), true),
            ("terminal_entries", "longtext", None, true),
            ("record_format", "varchar", Some(32), false),
        ],
    )
    .await?;
    validate_index(
        connection,
        "session_catalog",
        "PRIMARY",
        true,
        &["session_id"],
    )
    .await?;
    validate_index(
        connection,
        "session_catalog",
        "session_catalog_created_order",
        true,
        &["created_order", "session_id"],
    )
    .await?;
    validate_index(
        connection,
        "session_catalog",
        "session_catalog_lifecycle_order",
        false,
        &["lifecycle_state", "updated_order", "session_id"],
    )
    .await?;
    validate_index(
        connection,
        "session_public_turns",
        "PRIMARY",
        true,
        &["node_id"],
    )
    .await?;
    validate_index(
        connection,
        "session_public_turns",
        "session_public_turn_identity",
        true,
        &["origin_session_id", "turn_id", "record_kind"],
    )
    .await?;
    validate_index(
        connection,
        "session_public_turns",
        "session_public_turn_predecessor",
        false,
        &["predecessor_node_id"],
    )
    .await?;
    validate_index(
        connection,
        "session_public_turns",
        "session_public_turn_continuation",
        false,
        &["continuation_of_node_id"],
    )
    .await?;
    Ok(())
}

async fn validate_session_provenance_shape(
    connection: &mut MySqlConnection,
    version: i32,
) -> Result<()> {
    bounded_query(
        sqlx::query("SELECT session_id FROM messages LIMIT 0").fetch_all(&mut *connection),
    )
    .await?;
    let columns = bounded_query(
        sqlx::query("SELECT data_type, is_nullable, character_maximum_length FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'messages' AND BINARY column_name = BINARY 'session_id'")
            .fetch_all(&mut *connection),
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
    bounded_query(sqlx::query(summary_projection).fetch_all(&mut *connection)).await?;
    bounded_query(
        sqlx::query("SELECT actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors LIMIT 0")
            .fetch_all(&mut *connection),
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
    validate_columns(connection, "context_summaries", &summary_columns).await?;
    validate_columns(
        connection,
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
        connection,
        "messages",
        "messages_namespace_session_sequence",
        false,
        &["namespace", "session_id", "sequence"],
    )
    .await?;
    validate_index(
        connection,
        "context_summaries",
        "PRIMARY",
        true,
        &["summary_id"],
    )
    .await?;
    validate_index(
        connection,
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
        connection,
        "context_summary_cursors",
        "PRIMARY",
        true,
        &["actor_namespace", "session_id", "source_namespace"],
    )
    .await?;
    Ok(())
}

async fn validate_columns(
    connection: &mut MySqlConnection,
    table: &str,
    expected: &[(&str, &str, Option<i64>, bool)],
) -> Result<()> {
    let rows = bounded_query(
        sqlx::query("SELECT column_name, data_type, is_nullable, character_maximum_length FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY ? ORDER BY ordinal_position")
            .bind(table)
            .fetch_all(&mut *connection),
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
    connection: &mut MySqlConnection,
    table: &str,
    index: &str,
    unique: bool,
    expected_columns: &[&str],
) -> Result<()> {
    let rows = bounded_query(
        sqlx::query("SELECT column_name, non_unique FROM information_schema.statistics WHERE table_schema = DATABASE() AND BINARY table_name = BINARY ? AND BINARY index_name = BINARY ? ORDER BY seq_in_index")
            .bind(table)
            .bind(index)
            .fetch_all(&mut *connection),
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

async fn validate_operation_receipt_shape(connection: &mut MySqlConnection) -> Result<()> {
    bounded_query(
        sqlx::query(
            "SELECT receipt_format, method, request_digest, result_ref FROM operations LIMIT 0",
        )
        .fetch_all(&mut *connection),
    )
    .await?;
    let rows = bounded_query(
        sqlx::query("SELECT column_name, data_type, is_nullable, column_default, character_maximum_length, character_set_name, collation_name FROM information_schema.columns WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'operations' AND BINARY column_name IN (BINARY 'receipt_format', BINARY 'method', BINARY 'request_digest', BINARY 'result_ref')")
            .fetch_all(&mut *connection),
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

pub(super) async fn validate_current(pool: &MemoryPool) -> Result<()> {
    validate_current_with(REGISTRY, pool).await
}

async fn validate_current_with(registry: Registry, pool: &MemoryPool) -> Result<()> {
    let found = validate_supported_with(registry, pool).await?;
    ensure!(
        found == registry.current,
        "memory schema version {found} requires writable upgrade to {}",
        registry.current
    );
    inventory_with(registry, pool).await?;
    Ok(())
}

pub(super) async fn validate_active(pool: &MemoryPool) -> Result<()> {
    validate_active_with(REGISTRY, pool).await
}

/// [`validate_active`] without classifying retained attempts, for a caller
/// that classifies them itself: the template shape check does, so a copied
/// stage's engine validates with this and then runs that check.
pub(super) async fn validate_active_unclassified(pool: &MemoryPool) -> Result<()> {
    validate_current_with(REGISTRY, pool).await?;
    clean(pool).await
}

/// Validate a current-schema read-only main without treating unrelated working
/// data as a migration failure.
pub(super) async fn validate_inspection(pool: &MemoryPool) -> Result<()> {
    validate_current_with(REGISTRY, pool).await?;
    authority_working_set(pool).await?;
    classify_historical_attempts(REGISTRY, pool, REGISTRY.current).await
}

/// Validate a stopped staging database exactly at the version published in its
/// immutable ready marker. A supported older stage remains activatable, but it
/// cannot contain attempts for work its publishing binary had not completed.
pub(super) async fn validate_ready(pool: &MemoryPool) -> Result<i32> {
    validate_ready_with(REGISTRY, pool).await
}

async fn validate_ready_with(registry: Registry, pool: &MemoryPool) -> Result<i32> {
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
    classify_historical_attempts(registry, pool, found).await?;
    Ok(found)
}

async fn validate_active_with(registry: Registry, pool: &MemoryPool) -> Result<()> {
    validate_current_with(registry, pool).await?;
    clean(pool).await?;
    classify_historical_attempts(registry, pool, registry.current).await
}

async fn validate_receipts(
    registry: Registry,
    connection: &mut MySqlConnection,
    found: i32,
) -> Result<()> {
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
        .fetch_all(connection),
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

async fn clean(pool: &MemoryPool) -> Result<()> {
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
async fn authority_working_set(pool: &MemoryPool) -> Result<()> {
    let rows = bounded_query(
        sqlx::query(
            "SELECT table_name, staged, status FROM dolt_status WHERE BINARY table_name = BINARY 'kuru_schema' OR BINARY table_name = BINARY 'kuru_migrations' OR BINARY table_name = BINARY 'kuru_migration_publications' ORDER BY BINARY table_name, staged, BINARY status LIMIT 4",
        )
        .fetch_all(pool),
    )
    .await?;
    ensure!(
        rows.is_empty(),
        // Publication records are receipt authority: one message covers them.
        "Dolt memory schema or migration receipt authority has uncommitted changes"
    );
    Ok(())
}

async fn retained_failed_shape(pool: &MemoryPool, definition: &Definition) -> Result<bool> {
    retained_failed_shape_in(pool, WorkingSet::Session, definition).await
}

/// Whose working set a `dolt_status` read observes: the querying session's
/// own database, or one reserved branch named from any session. The branch
/// form reads the same system table the branch's own session reads, without
/// a connection (and so without an identity check) on that branch.
#[derive(Clone, Copy)]
enum WorkingSet<'a> {
    Session,
    Branch(&'a str),
}

impl WorkingSet<'_> {
    fn status_table(self) -> Result<String> {
        match self {
            Self::Session => Ok("dolt_status".to_owned()),
            Self::Branch(name) => {
                // Only names already accepted by `parse_attempt_in` reach here;
                // re-check the alphabet before placing one in SQL text.
                ensure!(
                    !name.is_empty()
                        && name.len() <= 128
                        && name.bytes().all(|byte| byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'_'),
                    "reserved migration branch is malformed"
                );
                Ok(format!("`{DATABASE}/{name}`.dolt_status"))
            }
        }
    }
}

async fn working_set_changes(main: &MemoryPool, working_set: WorkingSet<'_>) -> Result<i64> {
    bounded_query(
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {}",
            working_set.status_table()?
        )))
        .fetch_one(main),
    )
    .await
}

async fn retained_failed_shape_in(
    pool: &MemoryPool,
    working_set: WorkingSet<'_>,
    definition: &Definition,
) -> Result<bool> {
    let limit = i64::try_from(definition.failed_status.len() + 1)?;
    let rows = bounded_query(
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT table_name, staged, status FROM {} ORDER BY BINARY table_name, staged, BINARY status LIMIT ?",
            working_set.status_table()?
        )))
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

async fn inventory_with(registry: Registry, pool: &MemoryPool) -> Result<()> {
    inventory_in(registry, pool, RESERVED_PREFIX).await
}

async fn inventory_in(registry: Registry, pool: &MemoryPool, prefix: &str) -> Result<()> {
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

async fn reserved_names(pool: &MemoryPool) -> Result<Vec<String>> {
    reserved_names_in(pool, RESERVED_PREFIX).await
}

async fn reserved_names_in(pool: &MemoryPool, prefix: &str) -> Result<Vec<String>> {
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

async fn close_routed_pool(pool: Option<MemoryPool>) -> Result<()> {
    if let Some(pool) = pool {
        tokio::time::timeout(QUERY_TIMEOUT, pool.close())
            .await
            .context("migration fixture connection-pool close deadline exceeded")?;
    }
    Ok(())
}

async fn close_branch_pool(pool: &MemoryPool) -> Result<()> {
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
pub(super) async fn upgrade(server: &Server, main: &MemoryPool) -> Result<()> {
    upgrade_with(REGISTRY, server, main, &MigrationRunnerHooks::none()).await
}

#[cfg(test)]
pub(super) async fn upgrade_main_to_v3_fixture(server: &Server, main: &MemoryPool) -> Result<()> {
    const RELEASED_V3: Registry = Registry {
        current: 3,
        definitions: &[V2, V3],
    };
    upgrade_with(RELEASED_V3, server, main, &MigrationRunnerHooks::none()).await
}

/// The permanent usage branch is writable independently of main. Give its
/// staged attempts a separate owned namespace so a main migration attempt can never
/// be mistaken for an exact-base usage attempt (or vice versa).
pub(super) async fn upgrade_usage(server: &Server, usage: &MemoryPool) -> Result<()> {
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
    usage: &MemoryPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_in(USAGE_REGISTRY, server, usage, hooks, USAGE_RESERVED_PREFIX).await
}

pub(super) async fn validate_usage(usage: &MemoryPool) -> Result<()> {
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
        usage,
        USAGE_REGISTRY.current,
        USAGE_RESERVED_PREFIX,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
pub(super) async fn upgrade_with_hooks(
    server: &Server,
    main: &MemoryPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_with(REGISTRY, server, main, hooks).await
}

async fn upgrade_with(
    registry: Registry,
    server: &Server,
    main: &MemoryPool,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    upgrade_in(registry, server, main, hooks, RESERVED_PREFIX).await
}

async fn upgrade_in(
    registry: Registry,
    server: &Server,
    main: &MemoryPool,
    hooks: &MigrationRunnerHooks,
    prefix: &str,
) -> Result<()> {
    loop {
        let found = validate_supported_with(registry, main).await?;
        inventory_in(registry, main, prefix).await?;
        // The step that introduces publication records records every branch
        // this classification accepted as published, and nothing else.
        let classified = classify_historical_attempts_in(registry, main, found, prefix).await?;
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
        let published = classified.published.as_slice();
        let (branch, operation, ready) = discover_current_attempt_in(
            registry,
            server,
            main,
            definition,
            &StepBase {
                base: &base,
                published,
            },
            hooks,
            prefix,
        )
        .await?;
        let attempt_record = AttemptRecord {
            branch: &branch,
            base: &base,
            published,
        };
        if ready {
            let attempt = server.pool(&branch).await?;
            let inspected = async {
                let target = revision(&attempt).await?;
                validate_attempt(registry, &attempt, definition, operation, &attempt_record)
                    .await?;
                Ok(target)
            }
            .await;
            let target = after_cleanup(inspected, close_branch_pool(&attempt).await)?;
            publish(registry, main, definition, &branch, &base, &target, hooks).await?;
            // One completed migration step: each iteration publishes the
            // next higher version, so this is bounded by the registry.
            server.advance_open();
            continue;
        }
        let attempt = server.pool(&branch).await?;
        let built = async {
            hooks.reach(MigrationBoundary::BeforeDdl).await?;
            build_attempt(
                registry,
                &attempt,
                definition,
                operation,
                &attempt_record,
                hooks,
            )
            .await?;
            let target = revision(&attempt).await?;
            validate_attempt(registry, &attempt, definition, operation, &attempt_record).await?;
            Ok(target)
        }
        .await;
        let target = after_cleanup(built, close_branch_pool(&attempt).await)?;
        publish(registry, main, definition, &branch, &base, &target, hooks).await?;
        server.advance_open();
    }
}

async fn ensure_usage_branch_at_v4(main: &MemoryPool, base: &str) -> Result<()> {
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
            sqlx::query(BRANCH_CREATE)
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
    main: &MemoryPool,
    current: i32,
) -> Result<()> {
    classify_historical_attempts_in(registry, main, current, RESERVED_PREFIX)
        .await
        .map(|_| ())
}

/// A retained main attempt that classification accepted as a clean, published
/// step: the evidence a publication record is written from.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Published {
    version: i32,
    branch: String,
    /// The head's sole parent: the exact base the step was built on.
    base: String,
    operation: Uuid,
}

/// One `kuru_migration_publications` row, as stored.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Record {
    version: i32,
    branch: String,
    base: String,
    operation: String,
    definition_digest: String,
    record_format: i8,
}

impl Record {
    /// The record a published step carries.
    fn of(registry: Registry, published: &Published) -> Result<Self> {
        Ok(Self {
            version: published.version,
            branch: published.branch.clone(),
            base: published.base.clone(),
            operation: published.operation.hyphenated().to_string(),
            definition_digest: digest(registry.definition(published.version)?),
            record_format: PUBLICATION_RECORD_FORMAT,
        })
    }
}

/// Whether classification verifies a recorded branch by its record. Only the
/// parity and measurement fixtures ignore records, to reach the verdict full
/// classification gives the same store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Records {
    Use,
    #[cfg(test)]
    Ignore,
}

/// What one classification pass accepted and how.
#[derive(Debug, Default)]
struct Classification {
    /// Every clean completed branch accepted as published, by version.
    published: Vec<Published>,
    /// Branches accepted by their publication record.
    by_record: usize,
    /// Branches classified in full.
    full: usize,
}

/// Classify every retained attempt at or below `current` from the caller's
/// own pool. Branch refs, working sets, rows at a commit, parents and ancestry
/// are system-table and `AS OF` reads; full schema validation at a commit runs
/// the unchanged validator on one detached session switched to that commit.
/// No pool is opened on a branch or commit, so a retained branch whose
/// identity row differs from the opener's is still classified by its content.
async fn classify_historical_attempts_in(
    registry: Registry,
    main: &MemoryPool,
    current: i32,
    prefix: &str,
) -> Result<Classification> {
    classify_historical_attempts_with(registry, main, current, prefix, Records::Use).await
}

async fn classify_historical_attempts_with(
    registry: Registry,
    main: &MemoryPool,
    current: i32,
    prefix: &str,
    records: Records,
) -> Result<Classification> {
    let mut revisions = RevisionReader::default();
    let classified =
        classify_retained_attempts(registry, main, &mut revisions, current, prefix, records).await;
    after_cleanup(classified, revisions.close().await)
}

async fn classify_retained_attempts(
    registry: Registry,
    main: &MemoryPool,
    revisions: &mut RevisionReader,
    current: i32,
    prefix: &str,
    records: Records,
) -> Result<Classification> {
    let main_head = revision(main).await?;
    let refs = reserved_refs_in(main, prefix).await?;
    // Records exist on main from the step that introduces them; a usage
    // ledger's own attempts never carry one.
    let recorded =
        if records == Records::Use && prefix == RESERVED_PREFIX && current >= PUBLICATION_VERSION {
            verified_records(registry, main, current, &refs).await?
        } else {
            BTreeMap::new()
        };
    let mut classification = Classification::default();
    for reference in refs {
        let name = reference.name;
        let (target, operation) = parse_attempt_in(prefix, &name)?;
        ensure!(
            target <= current + 1,
            "reserved Dolt migration branch targets an out-of-order step"
        );
        if target > current {
            continue;
        }
        if let Some(record) = recorded.get(&name) {
            // `verified_records` bound this ref to its published head.
            ensure!(
                reference.hash != main_head || target == current,
                "historical Dolt migration branch unexpectedly names active main"
            );
            classification.by_record += 1;
            classification.published.push(Published {
                version: target,
                branch: name,
                base: record.base.clone(),
                operation,
            });
            continue;
        }
        classification.full += 1;
        let definition = registry.definition(target)?;
        let working_set = WorkingSet::Branch(&name);
        let head = branch_head(main, &name).await?;
        let head_version = version_as_of(main, &head).await?;
        let dirty = working_set_changes(main, working_set).await?;
        let (outcome, published) = if dirty == 0 {
            // A clean branch's working set is its head commit.
            revisions
                .validate(main, registry, &head, definition.to)
                .await?;
            let receipt = receipt_as_of(main, &head, definition.to).await?;
            ensure!(
                receipt == operation.hyphenated().to_string(),
                "historical Dolt migration receipt does not match its branch"
            );
            let parent = sole_parent(main, &head).await?;
            revisions
                .validate(main, registry, &parent, definition.from)
                .await?;
            (
                ancestor(main, &head).await? && ancestor(main, &parent).await?,
                Some(parent),
            )
        } else {
            ensure!(
                head_version == definition.from,
                "dirty historical Dolt migration branch has an unexpected schema"
            );
            revisions
                .validate(main, registry, &head, definition.from)
                .await?;
            ensure!(
                retained_failed_shape_in(main, working_set, definition).await?,
                "dirty historical Dolt migration branch has an unexpected working set"
            );
            (ancestor(main, &head).await?, None)
        };
        ensure!(
            outcome,
            "historical Dolt migration branch is not retained by active history"
        );
        ensure!(
            head != main_head || target == current,
            "historical Dolt migration branch unexpectedly names active main"
        );
        if let Some(base) = published {
            classification.published.push(Published {
                version: target,
                branch: name,
                base,
                operation,
            });
        }
    }
    classification.published.sort();
    ensure!(
        classification
            .published
            .windows(2)
            .all(|pair| pair[0].version < pair[1].version),
        "multiple retained Dolt migration branches are published for one step"
    );
    Ok(classification)
}

/// A reserved branch as `dolt_branches` lists it.
struct ReservedRef {
    name: String,
    hash: String,
    dirty: bool,
}

/// [`reserved_names_in`] with each branch's head and whether its working set
/// differs from that head.
async fn reserved_refs_in(pool: &MemoryPool, prefix: &str) -> Result<Vec<ReservedRef>> {
    let rows: Vec<(String, String, bool)> = bounded_query(
        sqlx::query_as(
            "SELECT name, hash, dirty FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? LIMIT 65",
        )
        .bind(i64::try_from(prefix.len())?)
        .bind(prefix)
        .fetch_all(pool),
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, hash, dirty)| ReservedRef { name, hash, dirty })
        .collect())
}

const RECORDS_QUERY: &str = "SELECT version, branch, base, operation, definition_digest, record_format FROM kuru_migration_publications";

fn records_from(rows: Vec<sqlx::mysql::MySqlRow>) -> Result<Vec<Record>> {
    ensure!(
        rows.len() <= DEFINITION_LIMIT,
        "Dolt migration publication records exceed the registry bound"
    );
    rows.iter()
        .map(|row| {
            Ok(Record {
                version: row.try_get(0)?,
                branch: row.try_get(1)?,
                base: row.try_get(2)?,
                operation: row.try_get(3)?,
                definition_digest: row.try_get(4)?,
                record_format: row.try_get(5)?,
            })
        })
        .collect()
}

/// Every publication record of the database `pool` serves, by version.
async fn records_in(pool: &MemoryPool) -> Result<Vec<Record>> {
    let rows = bounded_query(
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "{RECORDS_QUERY} ORDER BY version LIMIT ?"
        )))
        .bind(i64::try_from(DEFINITION_LIMIT + 1)?)
        .fetch_all(pool),
    )
    .await?;
    records_from(rows)
}

/// Every publication record at one commit, by version.
async fn records_as_of(pool: &MemoryPool, commit: &str) -> Result<Vec<Record>> {
    let rows = bounded_query(
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "{RECORDS_QUERY} AS OF '{}' ORDER BY version LIMIT ?",
            commit_hash(commit)?
        )))
        .bind(i64::try_from(DEFINITION_LIMIT + 1)?)
        .fetch_all(pool),
    )
    .await?;
    records_from(rows)
}

fn record_mismatch(detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("migration publication record does not match active history: {detail}")
}

/// `?, ?, ...` for `count` bind parameters; never data.
fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

/// Verify every publication record of main independently, from main's own
/// pool, and return the records by branch name. A record never vouches for
/// another: each is checked against main's receipts and the compiled
/// definition, and each present branch against its own ref, working set,
/// sole parent, ancestry and `AS OF` content. A record whose branch was
/// deleted keeps only its base's place in main's history to check. Any
/// disagreement fails closed; nothing falls back to full classification.
async fn verified_records(
    registry: Registry,
    main: &MemoryPool,
    current: i32,
    refs: &[ReservedRef],
) -> Result<BTreeMap<String, Record>> {
    let records = records_in(main).await?;
    let receipts: BTreeMap<i32, String> = bounded_query(
        sqlx::query_as("SELECT version, operation FROM kuru_migrations ORDER BY version LIMIT ?")
            .bind(i64::try_from(DEFINITION_LIMIT + 1)?)
            .fetch_all(main),
    )
    .await?
    .into_iter()
    .collect();
    let mut verified = BTreeMap::new();
    let mut heads = Vec::new();
    for record in records {
        ensure!(
            (2..=current).contains(&record.version),
            record_mismatch(format!(
                "record for schema {} is outside the store's schema",
                record.version
            ))
        );
        let definition = registry.definition(record.version).map_err(|_| {
            record_mismatch(format!(
                "record for schema {} names no registered step",
                record.version
            ))
        })?;
        let (target, operation) = parse_attempt(&record.branch).map_err(|_| {
            record_mismatch(format!(
                "record for schema {} names a malformed branch",
                record.version
            ))
        })?;
        ensure!(
            target == record.version && operation.hyphenated().to_string() == record.operation,
            record_mismatch(format!(
                "record for schema {} disagrees with its branch name",
                record.version
            ))
        );
        ensure!(
            receipts.get(&record.version) == Some(&record.operation),
            record_mismatch(format!(
                "record for schema {} disagrees with main's receipt",
                record.version
            ))
        );
        ensure!(
            record.definition_digest == digest(definition),
            record_mismatch(format!(
                "record for schema {} names another definition",
                record.version
            ))
        );
        ensure!(
            record.record_format == PUBLICATION_RECORD_FORMAT,
            record_mismatch(format!(
                "record for schema {} has format {}",
                record.version, record.record_format
            ))
        );
        commit_hash(&record.base).map_err(|_| {
            record_mismatch(format!(
                "record for schema {} names a malformed base",
                record.version
            ))
        })?;
        if let Some(reference) = refs
            .iter()
            .find(|reference| reference.name == record.branch)
        {
            ensure!(
                !reference.dirty,
                "published migration branch has uncommitted changes"
            );
            commit_hash(&reference.hash)?;
            heads.push((record.clone(), reference.hash.clone()));
        }
        verified.insert(record.branch.clone(), record);
    }
    // One batched read per axis: each head's parents, then which heads and
    // bases main's history holds.
    if !heads.is_empty() {
        let mut query = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(format!(
            "SELECT commit_hash, parent_hash FROM dolt_commit_ancestors WHERE commit_hash IN ({}) LIMIT ?",
            placeholders(heads.len())
        )));
        for (_, head) in &heads {
            query = query.bind(head.as_str());
        }
        let parents = bounded_query(
            query
                .bind(i64::try_from(2 * heads.len() + 1)?)
                .fetch_all(main),
        )
        .await?;
        for (record, head) in &heads {
            let found: Vec<&str> = parents
                .iter()
                .filter(|(commit, _)| commit == head)
                .map(|(_, parent)| parent.as_str())
                .collect();
            ensure!(
                found == [record.base.as_str()],
                record_mismatch(format!(
                    "branch for schema {} is not the sole child of its recorded base",
                    record.version
                ))
            );
        }
    }
    let commits: BTreeSet<&str> = heads
        .iter()
        .map(|(_, head)| head.as_str())
        .chain(verified.values().map(|record| record.base.as_str()))
        .collect();
    if !commits.is_empty() {
        let mut query = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
            "SELECT commit_hash FROM dolt_log WHERE commit_hash IN ({}) LIMIT ?",
            placeholders(commits.len())
        )));
        for commit in &commits {
            query = query.bind(*commit);
        }
        let found: BTreeSet<String> = bounded_query(
            query
                .bind(i64::try_from(commits.len() + 1)?)
                .fetch_all(main),
        )
        .await?
        .into_iter()
        .collect();
        for record in verified.values() {
            let head = heads
                .iter()
                .find(|(recorded, _)| recorded.branch == record.branch)
                .map(|(_, head)| head.as_str());
            ensure!(
                found.contains(&record.base) && head.is_none_or(|head| found.contains(head)),
                record_mismatch(format!(
                    "record for schema {} is outside main's history",
                    record.version
                ))
            );
        }
    }
    for (record, head) in &heads {
        let (version, operation): (Option<i32>, Option<String>) = bounded_query(
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT (SELECT version FROM kuru_schema AS OF '{0}' WHERE id = 1), (SELECT operation FROM kuru_migrations AS OF '{0}' WHERE version = ?)",
                commit_hash(head)?
            )))
            .bind(record.version)
            .fetch_one(main),
        )
        .await?;
        ensure!(
            version == Some(record.version)
                && operation.as_deref() == Some(record.operation.as_str()),
            record_mismatch(format!(
                "branch for schema {} does not carry its recorded schema and receipt",
                record.version
            ))
        );
    }
    Ok(verified)
}

/// Dolt's commit hash: 20 bytes as 32 characters of base32 `{0-9,a-v}`.
/// Neither `AS OF` nor `USE` accepts a placeholder, so every revision is
/// checked against this alphabet before it is placed in SQL text.
fn commit_hash(value: &str) -> Result<&str> {
    ensure!(
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte)),
        "Dolt migration revision is not a commit hash"
    );
    Ok(value)
}

async fn branch_head(main: &MemoryPool, name: &str) -> Result<String> {
    let heads: Vec<String> = bounded_query(
        sqlx::query_scalar("SELECT hash FROM dolt_branches WHERE BINARY name = BINARY ? LIMIT 2")
            .bind(name)
            .fetch_all(main),
    )
    .await?;
    ensure!(
        heads.len() == 1,
        "historical Dolt migration branch head is missing or ambiguous"
    );
    let head = heads.into_iter().next().expect("one head checked");
    commit_hash(&head)?;
    Ok(head)
}

async fn version_as_of(main: &MemoryPool, commit: &str) -> Result<i32> {
    bounded_query(
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM kuru_schema AS OF '{}' WHERE id = 1",
            commit_hash(commit)?
        )))
        .fetch_one(main),
    )
    .await
}

async fn receipt_as_of(main: &MemoryPool, commit: &str, version: i32) -> Result<String> {
    bounded_query(
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT operation FROM kuru_migrations AS OF '{}' WHERE version = ?",
            commit_hash(commit)?
        )))
        .bind(version)
        .fetch_one(main),
    )
    .await
}

/// One detached session from the caller's pool, switched with `USE` to one
/// immutable commit at a time so the unchanged `DATABASE()`-scoped schema
/// validator reads that commit exactly as a session opened on it would. It
/// never returns to the pool: `close` ends it, and any error ends the pass.
#[derive(Default)]
struct RevisionReader {
    connection: Option<MySqlConnection>,
}

impl RevisionReader {
    async fn validate(
        &mut self,
        source: &MemoryPool,
        registry: Registry,
        commit: &str,
        expected: i32,
    ) -> Result<()> {
        let database = format!("{DATABASE}/{}", commit_hash(commit)?);
        if self.connection.is_none() {
            self.connection = Some(acquire(source).await?.detach());
        }
        let connection = self
            .connection
            .as_mut()
            .expect("revision session acquired above");
        bounded_query(connection.execute(sqlx::AssertSqlSafe(format!("USE `{database}`")))).await?;
        let selected: Option<String> =
            bounded_query(sqlx::query_scalar("SELECT DATABASE()").fetch_one(&mut *connection))
                .await?;
        ensure!(
            selected.as_deref() == Some(database.as_str()),
            "Dolt migration revision session did not select its commit"
        );
        validate_version_on(registry, connection, expected).await
    }

    async fn close(self) -> Result<()> {
        if let Some(connection) = self.connection {
            bounded_query(connection.close()).await?;
        }
        Ok(())
    }
}

/// The classifier this module used before main-pool classification: a pool on
/// every retained branch and on its parent commit. Kept only as the parity
/// oracle for `main_pool_classification_agrees_with_branch_pool_classification`;
/// no product path reaches it.
#[cfg(test)]
async fn classify_with_branch_pools_in(
    registry: Registry,
    server: &Server,
    main: &MemoryPool,
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

async fn ancestor(main: &MemoryPool, ancestor: &str) -> Result<bool> {
    let count: i64 = bounded_query(
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?")
            .bind(ancestor)
            .fetch_one(main),
    )
    .await?;
    Ok(count == 1)
}

async fn sole_parent(pool: &MemoryPool, commit: &str) -> Result<String> {
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
    main: &MemoryPool,
    definition: &Definition,
    step: &StepBase<'_>,
    hooks: &MigrationRunnerHooks,
    prefix: &str,
) -> Result<(String, Uuid, bool)> {
    let StepBase { base, published } = *step;
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
                let record = AttemptRecord {
                    branch: &name,
                    base,
                    published,
                };
                validate_attempt(registry, &attempt, definition, operation, &record).await?;
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
    let (mut connection, connection_id) =
        super::owned_connection(command_pool, connection_deadline()).await?;
    let created = async {
        hooks.reach(MigrationBoundary::BeforeBranch).await?;
        bounded_query(
            sqlx::query(BRANCH_CREATE)
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

/// The exact base a step builds on and the publications the same open's
/// classification accepted.
#[derive(Clone, Copy)]
struct StepBase<'a> {
    base: &'a str,
    published: &'a [Published],
}

/// What an attempt at or above [`PUBLICATION_VERSION`] records: its own
/// branch and exact base, and for the step that introduces records, every
/// branch the same open's classification accepted as published.
struct AttemptRecord<'a> {
    branch: &'a str,
    base: &'a str,
    published: &'a [Published],
}

impl AttemptRecord<'_> {
    /// Every record the attempt's commit must hold, by version: the base's
    /// own records (or, for the introducing step, the backfill) and its own.
    async fn expected(
        &self,
        registry: Registry,
        pool: &MemoryPool,
        definition: &Definition,
        operation: Uuid,
    ) -> Result<Vec<Record>> {
        let mut expected = if definition.to == PUBLICATION_VERSION {
            self.backfill(registry, definition)?
        } else {
            records_as_of(pool, self.base).await?
        };
        expected.push(self.own(registry, definition, operation)?);
        expected.sort();
        Ok(expected)
    }

    fn own(&self, registry: Registry, definition: &Definition, operation: Uuid) -> Result<Record> {
        Record::of(
            registry,
            &Published {
                version: definition.to,
                branch: self.branch.to_owned(),
                base: self.base.to_owned(),
                operation,
            },
        )
    }

    /// The introducing step's records of earlier published branches.
    fn backfill(&self, registry: Registry, definition: &Definition) -> Result<Vec<Record>> {
        self.published
            .iter()
            .map(|published| {
                ensure!(
                    published.version < definition.to,
                    "Dolt migration backfill names a step at or after its own"
                );
                Record::of(registry, published)
            })
            .collect()
    }
}

async fn build_attempt(
    registry: Registry,
    pool: &MemoryPool,
    definition: &Definition,
    operation: Uuid,
    record: &AttemptRecord<'_>,
    hooks: &MigrationRunnerHooks,
) -> Result<()> {
    let records = if definition.to >= PUBLICATION_VERSION {
        let mut records = if definition.to == PUBLICATION_VERSION {
            record.backfill(registry, definition)?
        } else {
            Vec::new()
        };
        records.push(record.own(registry, definition, operation)?);
        records
    } else {
        Vec::new()
    };
    let source_revision = if definition.to == V7.to {
        Some(revision(pool).await?)
    } else {
        None
    };
    let routed = routed_pool(pool, hooks, MigrationBoundary::BeforeCommit).await?;
    let command_pool = routed.as_ref().unwrap_or(pool);
    let (mut connection, id) = super::owned_connection(command_pool, connection_deadline()).await?;
    let result = async {
        for statement in definition.sql {
            bounded_query(sqlx::query(*statement).execute(&mut connection)).await?;
        }
        hooks.reach(MigrationBoundary::AfterDdl).await?;
        bounded_query(sqlx::query(ATTEMPT_BEGIN).execute(&mut connection)).await?;
        if let Some(source_revision) = source_revision.as_deref() {
            migrate_legacy_session_catalog(&mut connection, source_revision).await?;
        }
        if definition.id == V10.id {
            topology_state::materialize(&mut connection).await?;
        }
        let advanced = bounded_query(
            sqlx::query(ATTEMPT_ADVANCE)
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
            sqlx::query(ATTEMPT_RECEIPT)
                .bind(definition.to)
                .bind(definition.id)
                .bind(digest(definition))
                .bind(operation.hyphenated().to_string())
                .execute(&mut connection),
        )
        .await?;
        for record in &records {
            bounded_query(
                sqlx::query(ATTEMPT_RECORD)
                    .bind(record.version)
                    .bind(&record.branch)
                    .bind(&record.base)
                    .bind(&record.operation)
                    .bind(&record.definition_digest)
                    .bind(record.record_format)
                    .execute(&mut connection),
            )
            .await?;
        }
        hooks.reach(MigrationBoundary::BeforeCommit).await?;
        let [before, middle, after] = ATTEMPT_MESSAGE;
        bounded_query(
            sqlx::query(ATTEMPT_COMMIT)
                .bind(format!(
                    "{before}{}{middle}{operation}{after}",
                    definition.to
                ))
                .bind(AUTHOR)
                .fetch_all(&mut connection),
        )
        .await?;
        bounded_query(sqlx::query(ATTEMPT_END).execute(&mut connection)).await?;
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

async fn migrate_legacy_session_catalog(
    connection: &mut sqlx::MySqlConnection,
    source_revision: &str,
) -> Result<()> {
    const MAX_LEGACY_SESSION_INDEX_BYTES: usize = 32 * 1024 * 1024;
    const MAX_LEGACY_SESSIONS: usize = 100_000;
    let candidates = bounded_query(
        sqlx::query("SELECT `key`, value FROM state WHERE BINARY `key` LIKE _binary '%/sessions' ORDER BY BINARY `key` LIMIT 65")
            .fetch_all(&mut *connection),
    )
    .await?;
    ensure!(
        candidates.len() <= 64,
        "legacy session-index candidates exceed the migration bound"
    );
    let mut indexes = Vec::new();
    for row in candidates {
        let key = String::from_utf8(row.try_get::<Vec<u8>, _>("key")?)
            .context("legacy session-index key is not UTF-8")?;
        let Some(scope) = key.strip_suffix("/sessions") else {
            continue;
        };
        if !valid_project_scope(scope) {
            continue;
        }
        let value: String = row.try_get("value")?;
        ensure!(
            value.len() <= MAX_LEGACY_SESSION_INDEX_BYTES,
            "legacy session index exceeds the migration byte bound"
        );
        indexes.push((scope.to_owned(), value));
    }
    ensure!(
        indexes.len() <= 1,
        "multiple durable project session indexes are ambiguous"
    );
    let Some((scope, value)) = indexes.pop() else {
        return Ok(());
    };
    let values: Vec<serde_json::Value> = match serde_json::from_str(&value) {
        Ok(values) => values,
        Err(_) => return Ok(()),
    };
    ensure!(
        values.len() <= MAX_LEGACY_SESSIONS,
        "legacy session index exceeds the migration row bound"
    );
    let mut decoded = Vec::with_capacity(values.len());
    let mut occurrences = std::collections::BTreeMap::<String, usize>::new();
    for value in values {
        let Ok(session) = serde_json::from_value::<LegacySession>(value) else {
            continue;
        };
        *occurrences.entry(session.id.clone()).or_default() += 1;
        decoded.push(session);
    }
    for (index, session) in decoded.into_iter().enumerate() {
        if occurrences.get(&session.id) != Some(&1) {
            continue;
        }
        let detail_key = format!("{scope}/session/{}", session.id);
        if detail_key.len() > 1024 {
            continue;
        }
        let detail: Option<String> = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE BINARY `key` = BINARY ? LIMIT 1")
                .bind(detail_key.as_bytes())
                .fetch_optional(&mut *connection),
        )
        .await?;
        let Some(detail) = detail else {
            continue;
        };
        let Ok(detail) = serde_json::from_str::<LegacySession>(&detail) else {
            continue;
        };
        if detail != session {
            continue;
        }
        let order = i64::try_from(index + 1).context("legacy session order exceeds SQL range")?;
        let transcript_namespace = format!("{scope}/transcript/{}", session.id);
        if transcript_namespace.len() > 1024 {
            continue;
        }
        let range: (i64, Option<i64>, Option<i64>, i64) = bounded_query(
            sqlx::query_as("SELECT COUNT(*), MIN(sequence), MAX(sequence), COUNT(CASE WHEN session_id IS NULL OR BINARY session_id = BINARY ? THEN 1 END) FROM messages WHERE BINARY namespace = BINARY ?")
                .bind(session.id.as_bytes())
                .bind(transcript_namespace.as_bytes())
                .fetch_one(&mut *connection),
        )
        .await?;
        if range.0 != range.3 {
            continue;
        }
        let legacy_prefix = match (range.0, range.1, range.2) {
            (0, None, None) => None,
            (count, Some(first), Some(through)) if count > 0 => Some(LegacyTranscriptPrefix {
                namespace: transcript_namespace,
                source_session_id: session.id.clone(),
                source_revision: source_revision.to_owned(),
                first_sequence: first,
                through_sequence: through,
                row_count: u64::try_from(count).context("legacy transcript count is negative")?,
                record_format: LEGACY_PREFIX_RECORD_FORMAT.into(),
            }),
            _ => continue,
        };
        let catalog = SessionCatalogRecord {
            session_id: session.id,
            mode: session.mode,
            label: session.label,
            created_order: u64::try_from(order).expect("positive legacy order"),
            updated_order: u64::try_from(order).expect("positive legacy order"),
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: None,
            pending_node_id: None,
            legacy_prefix,
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        };
        if validate_session_catalog(&catalog).is_err() {
            continue;
        }
        bounded_query(
            sqlx::query("INSERT INTO session_catalog (session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format) VALUES (?, ?, ?, ?, ?, ?, 'active', NULL, NULL, ?, NULL, ?)")
                .bind(catalog.session_id.as_bytes())
                .bind(catalog.mode.to_string())
                .bind(&catalog.label)
                .bind(order)
                .bind(order)
                .bind(0_i64)
                .bind(catalog.legacy_prefix.as_ref().map(serde_json::to_string).transpose()?)
                .bind(SESSION_CATALOG_RECORD_FORMAT)
                .execute(&mut *connection),
        )
        .await?;
    }
    Ok(())
}

fn valid_project_scope(scope: &str) -> bool {
    scope.strip_prefix("project/").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

async fn validate_attempt(
    registry: Registry,
    pool: &MemoryPool,
    definition: &Definition,
    operation: Uuid,
    record: &AttemptRecord<'_>,
) -> Result<()> {
    let base = record.base;
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
    if definition.to >= PUBLICATION_VERSION {
        let expected = record
            .expected(registry, pool, definition, operation)
            .await?;
        ensure!(
            records_in(pool).await? == expected,
            "Dolt migration attempt's publication records differ from its branch, base, receipt, definition or classified publications"
        );
    }
    Ok(())
}

async fn publish(
    registry: Registry,
    main: &MemoryPool,
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
    let (mut connection, id) = super::owned_connection(command_pool, connection_deadline()).await?;
    let result = async {
        hooks.reach(MigrationBoundary::BeforePublish).await?;
        bounded_query(
            sqlx::query(PUBLISH_MERGE)
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
mod main_pool_classification_tests;

#[cfg(test)]
mod publication_record_tests;

pub(super) mod template_shape;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct AbortOnDrop(tokio::task::AbortHandle);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct DurableSnapshot {
        head: String,
        refs: Vec<(String, String)>,
        status: Vec<(String, i64, String)>,
    }

    /// A test-only step after the real publication-record step: it records
    /// its own publication like every later main step.
    const V11: Definition = Definition {
        from: 10,
        to: 11,
        id: "kuru.memory.test-marker.v11",
        sql: &["CREATE TABLE kuru_migration_test_v11 (marker INT PRIMARY KEY)"],
        transform: "none",
        postcondition: "version=11;test marker table exists;v8 publication records remain exact",
        failed_status: &[StatusRow {
            table: "kuru_migration_test_v11",
            staged: 0,
            status: "new table",
        }],
    };
    const TEST_DEFINITIONS: &[Definition] = &[
        V2,
        super::V3,
        super::V4,
        super::V5,
        super::V6,
        super::V7,
        super::V8,
        super::V9,
        super::V10,
        V11,
    ];
    pub(super) const TEST_REGISTRY: Registry = Registry {
        current: 11,
        definitions: TEST_DEFINITIONS,
    };
    pub(super) const RELEASED_V9_REGISTRY: Registry = Registry {
        current: 9,
        definitions: &[
            V2,
            super::V3,
            super::V4,
            super::V5,
            super::V6,
            super::V7,
            super::V8,
            super::V9,
        ],
    };
    pub(super) const RELEASED_V8_REGISTRY: Registry = Registry {
        current: 8,
        definitions: &[
            V2,
            super::V3,
            super::V4,
            super::V5,
            super::V6,
            super::V7,
            super::V8,
        ],
    };
    pub(super) const RELEASED_V7_REGISTRY: Registry = Registry {
        current: 7,
        definitions: &[V2, super::V3, super::V4, super::V5, super::V6, super::V7],
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
    const RELEASED_V6_DEFINITIONS: &[Definition] =
        &[V2, super::V3, super::V4, super::V5, super::V6];
    const RELEASED_V6_REGISTRY: Registry = Registry {
        current: 6,
        definitions: RELEASED_V6_DEFINITIONS,
    };

    const RELEASED_V4_REGISTRY: Registry = Registry {
        current: 4,
        definitions: &[V2, super::V3, super::V4],
    };

    #[tokio::test]
    async fn older_registry_rejects_v4_store_without_mutating_it() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "4".repeat(64)),
        )
        .await?;
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
        let error = validate_active_with(RELEASED_V3_REGISTRY, &main)
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

    #[tokio::test]
    async fn v8_registry_rejects_current_store_without_mutating_it() -> Result<()> {
        let store = super::super::MemoryStore::temporary_cold().await?;
        let before = durable_snapshot(&store.pool).await?;
        let error = validate_active_with(RELEASED_V8_REGISTRY, &store.pool)
            .await
            .expect_err("a v8 binary must reject current memory before opening it for writes");
        assert!(
            format!("{error:#}").contains(&format!(
                "unsupported Dolt memory schema version {CURRENT_VERSION}"
            )),
            "unexpected older-registry refusal: {error:#}"
        );
        assert_eq!(durable_snapshot(&store.pool).await?, before);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn topology_state_v9_upgrade_materializes_then_latest_all_modes_and_retains_bytes()
    -> Result<()> {
        crate::test_support::closing(async {
        use serde_json::json;
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "a".repeat(64));
        let options =
            crate::test_support::warmed_open_options(root.path().join("private"), scope.clone())
                .await?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        crate::test_support::closing::register(server.clone());
        let main = server.pool("main").await?;
        upgrade_with(
            RELEASED_V9_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        let mut originals = Vec::new();
        let long_identity = format!("extra/slash/\u{1}/{}", "x".repeat(2048));
        for mode in Mode::ALL {
            let parts = kuru_core::Framework::builtin(mode).parts;
            let mut raw_parts = serde_json::to_value(&parts)?;
            raw_parts[0]["unknown_nested"] = json!({"retained":true});
            let mut archived = raw_parts[0].clone();
            archived["id"] = json!("archived-extra");
            archived["active"] = json!(false);
            raw_parts.as_array_mut().unwrap().push(archived);
            let relationships = vec![kuru_core::Relationship::new(
                kuru_core::RelationshipKind::Alliance,
                vec![parts[0].id.clone(), parts[1].id.clone()],
            )?];
            let mut topology = json!({"parts":raw_parts,"relationships":relationships,"states":{},"focus":{"id":parts[0].id,"remaining":3},"future_top_level":true});
            topology["states"][&long_identity] =
                json!({"activation":0.75,"note":"then latest","future_report":17});
            topology["states"]["archived-extra"] =
                json!({"activation":0.1,"note":"retained archived"});
            let raw = format!(" {} \n", serde_json::to_string(&topology)?);
            let key = format!("{scope}/{mode}/topology");
            bounded_query(
                sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, ?, 4)")
                    .bind(key.as_bytes())
                    .bind(&raw)
                    .execute(main.as_ref()),
            )
            .await?;
            originals.push((mode, key, raw, topology));
        }
        // Simulate a legacy writer after initial staging preparation: migration
        // must decode this latest durable blob, not an earlier inventory copy.
        originals[0].3["states"][&long_identity]["note"] = json!("last legacy writer");
        originals[0].2 = format!(" {} \n", serde_json::to_string(&originals[0].3)?);
        bounded_query(
            sqlx::query("UPDATE state SET value = ?, version = version + 1 WHERE `key` = ?")
                .bind(&originals[0].2)
                .bind(originals[0].1.as_bytes())
                .execute(main.as_ref()),
        )
        .await?;
        commit_fixture(&main, "Then-latest legacy topologies").await?;
        let historical = revision(&main).await?;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind("historical_schema9")
                .bind(&historical)
                .fetch_all(main.as_ref()),
        )
        .await?;
        // Equal materialized destinations are accepted without resetting a
        // held nonzero version token. Keep the historical ref before these rows.
        let equal_mode = originals[0].0;
        let equal_membership = json!({"record_format":"membership.v1","parts":originals[0].3["parts"],"relationships":originals[0].3["relationships"]});
        let suffix = Sha256::digest(long_identity.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let equal_report = json!({"record_format":"state_report.v1","identity":long_identity,"report":originals[0].3["states"][&long_identity]});
        for (key, value) in [
            (format!("{scope}/{equal_mode}/membership"), equal_membership),
            (format!("{scope}/{equal_mode}/state/{suffix}"), equal_report),
        ] {
            bounded_query(
                sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, ?, 7)")
                    .bind(key.as_bytes())
                    .bind(serde_json::to_string(&value)?)
                    .execute(main.as_ref()),
            ).await?;
        }
        commit_fixture(&main, "Equal preexisting topology destinations").await?;
        upgrade_with(REGISTRY, &server, &main, &MigrationRunnerHooks::none()).await?;
        assert_eq!(version(&main).await?, 10);
        for (mode, key, raw, topology) in originals {
            let original: String = bounded_query(
                sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                    .bind(key.as_bytes())
                    .fetch_one(main.as_ref()),
            )
            .await?;
            assert_eq!(original, raw);
            let membership: String = bounded_query(
                sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                    .bind(format!("{scope}/{mode}/membership").as_bytes())
                    .fetch_one(main.as_ref()),
            )
            .await?;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&membership)?,
                json!({"record_format":"membership.v1","parts":topology["parts"],"relationships":topology["relationships"]})
            );
            let member_version: i64 = bounded_query(
                sqlx::query_scalar("SELECT version FROM state WHERE `key` = ?")
                    .bind(format!("{scope}/{mode}/membership").as_bytes())
                    .fetch_one(main.as_ref()),
            ).await?;
            assert_eq!(member_version, if mode == equal_mode { 7 } else { 0 });
            let rows = bounded_query(
                sqlx::query("SELECT `key`, value, version FROM state WHERE LEFT(`key`, ?) = ?")
                    .bind(format!("{scope}/{mode}/state/").len() as i64)
                    .bind(format!("{scope}/{mode}/state/").as_bytes())
                    .fetch_all(main.as_ref()),
            )
            .await?;
            assert_eq!(rows.len(), 2);
            for row in rows {
                let record: serde_json::Value =
                    serde_json::from_str(&row.try_get::<String, _>("value")?)?;
                let identity = record["identity"].as_str().unwrap();
                assert_eq!(record["record_format"], "state_report.v1");
                assert_eq!(record["report"], topology["states"][identity]);
                let suffix = Sha256::digest(identity.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                assert_eq!(
                    row.try_get::<Vec<u8>, _>("key")?,
                    format!("{scope}/{mode}/state/{suffix}").into_bytes()
                );
                assert_eq!(
                    row.try_get::<i64, _>("version")?,
                    if mode == equal_mode && identity == long_identity { 7 } else { 0 }
                );
            }
        }
        let before = durable_snapshot(&main).await?;
        let refusal = validate_active_with(RELEASED_V9_REGISTRY, &main)
            .await
            .unwrap_err();
        assert!(format!("{refusal:#}").contains("unsupported Dolt memory schema version 10"));
        assert_eq!(durable_snapshot(&main).await?, before);
        let historical_pool = server.pool("historical_schema9").await?;
        assert_eq!(validate_historical(&historical_pool).await?, 9);
        let missing: i64 = bounded_query(
            sqlx::query_scalar("SELECT COUNT(*) FROM state WHERE RIGHT(`key`, 11) = ?")
                .bind(b"/membership".as_slice())
                .fetch_one(historical_pool.as_ref()),
        )
        .await?;
        assert_eq!(missing, 0);
        historical_pool.close().await;
        drop(historical_pool);
        main.close().await;
        drop(main);
        server.close().await
        }).await
    }

    #[tokio::test]
    async fn topology_state_v9_upgrade_refuses_malformed_or_conflicting_destinations_atomically()
    -> Result<()> {
        crate::test_support::closing(async {
            for refusal in ["membership", "report", "malformed"] {
                let malformed = refusal == "malformed";
                let root = crate::test_support::tempdir()?;
                let scope = format!("project/{}", "b".repeat(64));
                let options = crate::test_support::warmed_open_options(
                    root.path().join("private"),
                    scope.clone(),
                )
                .await?;
                super::super::tests::released_v1(&options).await?;
                let server = super::super::tests::released_server(&options).await?;
                crate::test_support::closing::register(server.clone());
                let main = server.pool("main").await?;
                upgrade_with(
                    RELEASED_V9_REGISTRY,
                    &server,
                    &main,
                    &MigrationRunnerHooks::none(),
                )
                .await?;
                for (mode, value) in [
                    (
                        "freudian",
                        r#"{"parts":[],"relationships":[],"states":{},"focus":null}"#,
                    ),
                    (
                        "ifs",
                        if malformed {
                            "{}"
                        } else {
                            r#"{"parts":[],"relationships":[],"states":{"extra/slash":{"activation":0.5,"note":"retained"}},"focus":null}"#
                        },
                    ),
                ] {
                    bounded_query(
                        sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, ?, 0)")
                            .bind(format!("{scope}/{mode}/topology").as_bytes())
                            .bind(value)
                            .execute(main.as_ref()),
                    )
                    .await?;
                }
                if !malformed {
                    let destination = if refusal == "report" {
                        let suffix = Sha256::digest(b"extra/slash")
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>();
                        format!("{scope}/ifs/state/{suffix}")
                    } else {
                        format!("{scope}/ifs/membership")
                    };
                    bounded_query(
                        sqlx::query(
                            "INSERT INTO state (`key`, value, version) VALUES (?, 'null', 7)",
                        )
                        .bind(destination.as_bytes())
                        .execute(main.as_ref()),
                    )
                    .await?;
                }
                commit_fixture(&main, "Refused topology migration source").await?;
                let before = durable_snapshot(&main).await?;
                let error = upgrade_with(REGISTRY, &server, &main, &MigrationRunnerHooks::none())
                    .await
                    .unwrap_err();
                let diagnostic = format!("{error:#}");
                assert!(
                    diagnostic.contains(if malformed {
                        "invalid legacy mode topology"
                    } else {
                        "conflicting topology migration destination"
                    }),
                    "unexpected migration refusal: {diagnostic}"
                );
                assert_eq!(revision(&main).await?, before.head);
                assert_eq!(version(&main).await?, 9);
                let partial: i64 = bounded_query(
                    sqlx::query_scalar("SELECT COUNT(*) FROM state WHERE `key` = ?")
                        .bind(format!("{scope}/freudian/membership").as_bytes())
                        .fetch_one(main.as_ref()),
                )
                .await?;
                assert_eq!(partial, 0);
                let receipts: i64 = bounded_query(
                    sqlx::query_scalar("SELECT COUNT(*) FROM kuru_migrations WHERE version = 10")
                        .fetch_one(main.as_ref()),
                )
                .await?;
                assert_eq!(receipts, 0);
                // The retained exact-source attempt is clean, and retry refuses the
                // same input rather than accepting a partially materialized graph.
                let retry = upgrade_with(REGISTRY, &server, &main, &MigrationRunnerHooks::none())
                    .await
                    .unwrap_err();
                assert!(format!("{retry:#}").contains(if malformed {
                    "invalid legacy mode topology"
                } else {
                    "conflicting topology migration destination"
                }));
                main.close().await;
                drop(main);
                server.close().await?;
            }
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn conditional_state_v8_upgrade_preserves_bytes_and_historical_contract() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "d".repeat(64)),
        )
        .await?;
        super::super::tests::released_v1(&options).await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        let legacy = r#"{ "parts": [], "unknown": { "future": true } }"#;
        let prepared = async {
            upgrade_with(
                RELEASED_V8_REGISTRY,
                &server,
                &main,
                &MigrationRunnerHooks::none(),
            )
            .await?;
            bounded_query(
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                    .bind(b"topology".as_slice())
                    .bind(legacy)
                    .execute(main.as_ref()),
            )
            .await?;
            commit_fixture(&main, "Preserve legacy topology bytes").await?;
            bounded_query(
                sqlx::query("CALL DOLT_BRANCH(?, ?)")
                    .bind("historical_schema8")
                    .bind(revision(&main).await?)
                    .fetch_all(main.as_ref()),
            )
            .await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        main.close().await;
        after_cleanup(prepared, server.close().await)?;
        let store = super::super::MemoryStore::open(options).await?;
        let verified = async {
            let raw: String = bounded_query(
                sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                    .bind(b"topology".as_slice())
                    .fetch_one(store.pool.as_ref()),
            )
            .await?;
            assert_eq!(raw, legacy);
            assert_eq!(store.get_versioned("topology").await?.unwrap().version, 0);
            let before = durable_snapshot(&store.pool).await?;
            assert!(
                validate_active_with(RELEASED_V8_REGISTRY, &store.pool)
                    .await
                    .is_err()
            );
            assert_eq!(durable_snapshot(&store.pool).await?, before);
            let old_pool = store.shared.server.pool("historical_schema8").await?;
            let old = super::super::MemoryStore {
                shared: store.shared.clone(),
                pool: old_pool.clone(),
                branch: "historical_schema8".into(),
                logical_receipt: None,
            };
            let before = durable_snapshot(&old_pool).await?;
            assert_eq!(
                old.get_many(&["topology".into()]).await?,
                vec![("topology".into(), Some(serde_json::from_str(legacy)?))]
            );
            assert!(old.get_versioned("topology").await.is_err());
            assert!(old.get_many_versioned(&["topology".into()]).await.is_err());
            assert!(
                old.put_many_conditional(
                    &[("topology".into(), crate::StateExpectation::Version(0))],
                    &[("topology".into(), serde_json::json!({}))]
                )
                .await
                .is_err()
            );
            assert_eq!(durable_snapshot(&old_pool).await?, before);
            old_pool.close().await;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        after_cleanup(verified, store.close().await)
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "c".repeat(64)),
        )
        .await?;
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "9".repeat(64)),
        )
        .await?;
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

    pub(super) async fn durable_snapshot(pool: &MemoryPool) -> Result<DurableSnapshot> {
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

    pub(super) async fn commit_fixture(pool: &MemoryPool, message: &str) -> Result<()> {
        bounded_query(
            sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                .bind(message)
                .bind(AUTHOR)
                .fetch_all(pool),
        )
        .await?;
        clean(pool).await
    }

    pub(super) async fn assert_failed_runner_unchanged(
        registry: Registry,
        server: &Server,
        main: &MemoryPool,
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

    pub(super) async fn snapshot_attempts(
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

    pub(super) async fn assert_attempts_unchanged(
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
        let future_version = CURRENT_VERSION + 1;
        let future = attempt_name(future_version, Uuid::nil());
        assert_eq!(
            parse_attempt(&future).unwrap(),
            (future_version, Uuid::nil())
        );
        assert!(REGISTRY.definition(future_version).is_err());
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
            let options = crate::test_support::warmed_open_options(
                root.path().join("private"),
                format!("project/{index:064x}"),
            )
            .await?;
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
                        sqlx::query("INSERT INTO kuru_migrations VALUES (?, ?, ?, ?)")
                            .bind(CURRENT_VERSION + 1)
                            .bind(format!("fixture.extra.v{}", CURRENT_VERSION + 1))
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
            let store = super::super::MemoryStore::temporary_cold().await?;
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
                let name = attempt_name(TEST_REGISTRY.current, operation);
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
                            &V11,
                            *operation,
                            &AttemptRecord {
                                branch: name,
                                base: &base,
                                published: &[],
                            },
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
                        &V11,
                        mismatched,
                        &AttemptRecord {
                            branch: &attempts[0].0,
                            base: &base,
                            published: &[],
                        },
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "6".repeat(64)),
        )
        .await?;
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
        validate_version_with(TEST_REGISTRY, &main, CURRENT_VERSION).await?;

        let base = revision(&main).await?;
        let mut names = Vec::with_capacity(INVENTORY_LIMIT);
        let existing = reserved_names(&main).await?.len();
        for _ in existing..INVENTORY_LIMIT {
            let name = attempt_name(TEST_REGISTRY.current, Uuid::new_v4());
            bounded_query(
                sqlx::query("CALL DOLT_BRANCH(?, ?)")
                    .bind(&name)
                    .bind(&base)
                    .fetch_all(main.as_ref()),
            )
            .await?;
            let attempt = server.pool(&name).await?;
            let prepared = bounded_query(sqlx::query(V11.sql[0]).execute(attempt.as_ref())).await;
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
            let store = super::super::MemoryStore::temporary_cold().await?;
            let base = store.revision().await?;
            let names = match invalid {
                InvalidInventory::Malformed => vec!["kuru_migration_bad".to_owned()],
                InvalidInventory::UnknownTarget => {
                    vec![attempt_name(TEST_REGISTRY.current + 1, Uuid::new_v4())]
                }
                InvalidInventory::Excess => {
                    let existing = reserved_names(&store.pool).await?.len();
                    ensure!(
                        existing <= INVENTORY_LIMIT,
                        "fixture inventory is already excessive"
                    );
                    (0..=(INVENTORY_LIMIT - existing))
                        .map(|_| attempt_name(TEST_REGISTRY.current, Uuid::new_v4()))
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
    async fn test_v11_receipt_order_and_operation_uniqueness_are_enforced() -> Result<()> {
        for repeated_operation in [false, true] {
            let store = super::super::MemoryStore::temporary_cold().await?;
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
                    sqlx::query("UPDATE kuru_migrations SET operation = ? WHERE version = 11")
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "b".repeat(64)),
        )
        .await?;
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
        let store = super::super::MemoryStore::temporary_cold().await?;
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "a".repeat(64)),
        )
        .await?;
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
        let record = AttemptRecord {
            branch: &non_ancestral_name,
            base: &divergent_head,
            published: &[],
        };
        build_attempt(
            REGISTRY,
            &non_ancestral,
            &V2,
            operation,
            &record,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        validate_attempt(REGISTRY, &non_ancestral, &V2, operation, &record).await?;
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

        let store = super::super::MemoryStore::temporary_cold().await?;
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
        // The advanced branch is recorded: its head is no longer the sole
        // child of the recorded base, so its record fails closed.
        let expected = "not the sole child of its recorded base".to_owned();
        assert_failed_runner_unchanged(
            TEST_REGISTRY,
            &store.shared.server,
            &store.pool,
            &before,
            &expected,
        )
        .await?;
        let completed = store.shared.server.pool(&completed_name).await?;
        assert_eq!(durable_snapshot(&completed).await?, completed_before);
        completed.close().await;
        drop(completed);
        store.close().await?;

        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "f".repeat(64)),
        )
        .await?;
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
    async fn retained_v2_attempts_and_candidate_survive_test_v11_progression() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let mut options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "3".repeat(64)),
        )
        .await?;
        // The test pools a retained attempt branch, which carries the store's
        // own identity only in a cold store; a template copy's branches carry
        // the placeholder.
        options.creation = super::super::Creation::Cold;
        let store = super::super::MemoryStore::open(options.clone()).await?;
        store
            .append("conversation", "user", "written after the v2 upgrade")
            .await?;
        let candidate = store.begin_candidate("pre-v11 candidate").await?;
        candidate
            .view()
            .append(
                "candidate",
                "assistant",
                "kept on current pre-future schema",
            )
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
            validate_ready_with(TEST_REGISTRY, &store.pool).await?,
            CURRENT_VERSION
        );
        let v11_operation = Uuid::new_v4();
        let v11_name = attempt_name(11, v11_operation);
        let current_base = store.revision().await?;
        bounded_query(
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(&v11_name)
                .bind(&current_base)
                .fetch_all(store.pool.as_ref()),
        )
        .await?;
        assert!(
            validate_ready_with(TEST_REGISTRY, &store.pool)
                .await
                .is_err(),
            "a ready v11 stage must reject while an earlier failed attempt remains"
        );

        upgrade_with(
            TEST_REGISTRY,
            &store.shared.server,
            &store.pool,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        validate_active_with(TEST_REGISTRY, &store.pool).await?;
        assert_eq!(version(&store.pool).await?, 11);
        let v11_attempt = store.shared.server.pool(&v11_name).await?;
        assert_eq!(revision(&v11_attempt).await?, store.revision().await?);
        let v11_receipt: String = bounded_query(
            sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = 11")
                .fetch_one(v11_attempt.as_ref()),
        )
        .await?;
        assert_eq!(v11_receipt, v11_operation.hyphenated().to_string());
        v11_attempt.close().await;
        drop(v11_attempt);
        assert_eq!(
            reserved_names(&store.pool)
                .await?
                .into_iter()
                .filter(|name| parse_attempt(name).is_ok_and(|(target, _)| target == 11))
                .count(),
            1,
            "the pristine exact-base v11 attempt must be reused"
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
            CURRENT_VERSION
        );
        assert_eq!(
            old_view.history("candidate", 10).await?[0].plain_text(),
            Some("kept on current pre-future schema")
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
            "pre-v11 candidate unexpectedly fast-forwarded into v11 main"
        );
        assert_eq!(durable_snapshot(&store.pool).await?, before_stale_merge);

        // Model a future v11 writer through its test registry and SQL view.
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
        assert_eq!(validate_supported_with(TEST_REGISTRY, &fresh).await?, 11);
        bounded_query(
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(b"post-v11".as_slice())
                .bind(json!({"preserved": true}).to_string())
                .execute(fresh.as_ref()),
        )
        .await?;
        commit_fixture(&fresh, "Test future-v11 candidate write").await?;
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
            .bind("written after schema v11")
            .execute(store.pool.as_ref()),
        )
        .await?;
        commit_fixture(&store.pool, "Test future-v11 conversation write").await?;
        validate_active_with(TEST_REGISTRY, &store.pool).await?;
        assert_eq!(version(&store.pool).await?, 11);
        let promoted_value: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(b"post-v11".as_slice())
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
        validate_active_with(TEST_REGISTRY, &main).await?;
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
            ["written after the v2 upgrade", "written after schema v11"]
        );
        let value: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(b"post-v11".as_slice())
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
            CURRENT_VERSION
        );
        let old_content: String = bounded_query(
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"candidate".as_slice())
                .fetch_one(old_candidate.as_ref()),
        )
        .await?;
        assert_eq!(old_content, "kept on current pre-future schema");
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
        let options = crate::test_support::warmed_open_options(
            root.path().join("private"),
            format!("project/{}", "a".repeat(64)),
        )
        .await?;
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
        // D (unit 7 T7): the pre-upgrade scan decodes the one owned row; the
        // receipt upgrade leaves `state` byte-identical, so no second walk
        // runs, and one record commit sits on the migration head.
        assert_eq!(
            store
                .shared
                .usage_open
                .lock()
                .expect("usage open lock")
                .clone(),
            Some(super::super::usage_ledger::UsageOpen {
                bound: false,
                scanned: Some(1),
                rescanned: None,
                recorded: true,
            })
        );
        let (record_head, record_message): (String, String) = bounded_query(
            sqlx::query_as("SELECT commit_hash, message FROM dolt_log LIMIT 1")
                .fetch_one(usage.as_ref()),
        )
        .await?;
        assert_eq!(record_head, revision(&usage).await?);
        assert!(
            record_message.starts_with("usage ledger validation v1\n\nKuru-Usage-State: "),
            "{record_message:?}"
        );
        let migration_head = sole_parent(&usage, &record_head).await?;
        let migration_message: String = bounded_query(
            sqlx::query_scalar("SELECT message FROM dolt_log WHERE commit_hash = ?")
                .bind(&migration_head)
                .fetch_one(usage.as_ref()),
        )
        .await?;
        assert!(
            migration_message.starts_with("Upgrade Kuru memory schema 4"),
            "{migration_message:?}"
        );
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
    async fn released_v5_summary_identity_survives_v6_v7_upgrade_reopen_and_export() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let options =
            crate::test_support::warmed_open_options(root.path().join("private"), scope.clone())
                .await?;
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
        let legacy_session = json!({
            "id": "legacy-session",
            "mode": "freudian",
            "turns": 1,
            "label": "legacy label",
            "last_completed_speaker": "Desire"
        });
        let legacy_index = serde_json::to_string(&vec![legacy_session.clone()])?;
        let legacy_detail = serde_json::to_string(&legacy_session)?;
        let legacy_turn_id = "legacy-safe-turn";
        let legacy_journal_key = format!("{scope}/session/legacy-session/turn/{}", "a".repeat(64));
        let legacy_pending_journal = json!({
            "format": 2,
            "id": legacy_turn_id,
            "prompt": "legacy visible transcript",
            "target": null,
            "transitions": ["Started", "Interrupted"],
            "possible_dispatch": false,
            "interruption_marker": true,
            "output": null
        });
        let ambiguous_value = r#"{"legacy":{"bytes":"retained exactly"}}"#;
        for (key, value) in [
            (format!("{scope}/sessions"), legacy_index),
            (format!("{scope}/session/legacy-session"), legacy_detail),
            (
                legacy_journal_key.clone(),
                serde_json::to_string(&legacy_pending_journal)?,
            ),
            (
                format!("{scope}/session/ambiguous-only"),
                ambiguous_value.to_owned(),
            ),
        ] {
            bounded_query(
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                    .bind(key.as_bytes())
                    .bind(value)
                    .execute(main.as_ref()),
            )
            .await?;
        }
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, session_id, role, content_format, content) VALUES (?, ?, ?, 'text-v1', ?)")
                .bind(format!("{scope}/transcript/legacy-session").into_bytes())
                .bind(b"legacy-session".as_slice())
                .bind(b"user".as_slice())
                .bind("legacy visible transcript")
                .execute(main.as_ref()),
        )
        .await?;
        bounded_query(
            sqlx::query("INSERT INTO messages (namespace, session_id, role, content_format, content) VALUES (?, NULL, ?, 'text-v1', ?)")
                .bind(format!("{scope}/transcript/ambiguous-only").into_bytes())
                .bind(b"assistant".as_slice())
                .bind("ambiguous transcript retained")
                .execute(main.as_ref()),
        )
        .await?;
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
        upgrade_with(
            RELEASED_V6_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        assert_eq!(version(&main).await?, 6);
        let v6_head = revision(&main).await?;
        main.close().await;
        drop(main);
        server.close().await?;

        let store = super::super::MemoryStore::open(options.clone()).await?;
        assert_eq!(version(&store.pool).await?, CURRENT_VERSION);
        assert_ne!(store.revision().await?, v6_head);
        assert_eq!(
            bounded_query(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_catalog")
                    .fetch_one(store.pool.as_ref()),
            )
            .await?,
            1
        );
        assert_eq!(
            bounded_query(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_public_turns")
                    .fetch_one(store.pool.as_ref()),
            )
            .await?,
            0
        );
        let catalog_page = store.session_catalog_page(None, None, None, 16).await?;
        assert_eq!(catalog_page.view, "main");
        assert_eq!(catalog_page.revision, store.revision().await?);
        assert_eq!(catalog_page.total_rows, 1);
        assert_eq!(catalog_page.records.len(), 1);
        assert!(catalog_page.next.is_none());
        assert_eq!(
            store
                .session_catalog_page(Some(SessionLifecycleState::Removed), None, None, 0)
                .await?
                .total_rows,
            0
        );
        let migrated: (Vec<u8>, String, String, i64, i64, i64, String, String) = bounded_query(
            sqlx::query_as("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, legacy_prefix FROM session_catalog LIMIT 1")
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(migrated.0, b"legacy-session");
        assert_eq!(migrated.1, "freudian");
        assert_eq!(migrated.2, "legacy label");
        assert_eq!((migrated.3, migrated.4, migrated.5), (1, 1, 0));
        assert_eq!(migrated.6, "active");
        let prefix: LegacyTranscriptPrefix = serde_json::from_str(&migrated.7)?;
        assert_eq!(
            prefix.namespace,
            format!("{scope}/transcript/legacy-session")
        );
        assert_eq!(prefix.source_session_id, "legacy-session");
        assert_eq!(prefix.source_revision, v6_head);
        assert_eq!(prefix.row_count, 1);
        let transcript = store
            .public_transcript_page("legacy-session", None, 16)
            .await?;
        assert_eq!(transcript.view, "main");
        assert_eq!(transcript.revision, store.revision().await?);
        assert_eq!(transcript.total_rows, 1);
        assert!(transcript.pending.is_none());
        assert!(transcript.next.is_none());
        assert!(matches!(
            transcript.records.as_slice(),
            [super::super::PublicTranscriptEntry::Legacy { message, .. }]
                if message.text_projection() == "legacy visible transcript"
        ));
        let legacy_resumed_journal = json!({
            "format": 2,
            "id": legacy_turn_id,
            "prompt": "legacy visible transcript",
            "target": null,
            "transitions": ["Started", "Interrupted", "Resumed"],
            "possible_dispatch": false,
            "interruption_marker": true,
            "output": null
        });
        store.close().await?;
        let _gate = crate::spawn_gate::spawning().await;
        // This fixture has no outer deadline; the helper retires the owner on
        // every exit path, and a failed retirement's verdict is attached to
        // the error below instead of replacing it.
        let settled = crate::test_support::serve_without_deadline(
            async |served| {
                let owner = crate::service::ServiceOwner::open(options.clone(), &project).await?;
                served.serve(owner)?;
                let executable = std::env::current_exe()?;
                let open = || {
                    crate::MemoryStore::open_managed_observed(
                        options.clone(),
                        project.clone(),
                        executable.clone(),
                    )
                    .1
                };
                let managed = open().await?;
                let sibling = open().await?;
                let barrier = crate::test_support::ReplyBarrier::default();
                managed.fixture_pause_next_service_reply(&barrier).await?;
                let mut resume = tokio::spawn({
                    let managed = managed.clone();
                    let namespace = format!("{scope}/transcript/legacy-session");
                    let journal_key = legacy_journal_key.clone();
                    let expected_journal = legacy_pending_journal.clone();
                    let prefix = prefix.clone();
                    async move {
                        managed
                            .checkpoint_session_turn(
                                &namespace,
                                "legacy-session",
                                &[],
                                &[(journal_key.clone(), legacy_resumed_journal)],
                                &super::super::SessionTurnCheckpoint::Resume {
                                    expected_generation: 0,
                                    turn_id: "legacy-safe-turn".into(),
                                    legacy: Some(super::super::LegacySessionTurnResume {
                                        legacy_prefix: prefix,
                                        journal_key,
                                        expected_journal,
                                    }),
                                },
                            )
                            .await
                    }
                });
                let _resume_cleanup = AbortOnDrop(resume.abort_handle());
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    tokio::select! {
                        () = barrier.wait_sent() => Ok(()),
                        result = &mut resume => bail!("migrated safe-journal resume completed before reply pause: {result:?}"),
                    }
                })
                .await
                .context("migrated safe-journal resume frame was not flushed")??;
                // The owner commits the accepted resume within its write
                // budget (`QUERY_TIMEOUT`, taken by `write_deadline`).
                tokio::time::timeout(QUERY_TIMEOUT, async {
                    loop {
                        let page = sibling
                            .public_transcript_page("legacy-session", None, 16)
                            .await?;
                        if matches!(page.pending.as_ref(), Some(record)
                            if record.kind == super::super::PublicTurnKind::LegacyContinuation
                                && record.turn_id == legacy_turn_id)
                        {
                            break Ok::<(), anyhow::Error>(());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                })
                .await
                .context("migrated safe-journal resume was not committed before reply loss")??;
                resume.abort();
                ensure!(
                    tokio::time::timeout(std::time::Duration::from_secs(5), resume)
                        .await
                        .context("cancelled migrated safe-journal resume did not end")?
                        .is_err_and(|error| error.is_cancelled()),
                    "migrated safe-journal resume completed before its reply was lost"
                );
                ensure!(managed.reconcile().await? == Some(true));
                managed.close().await?;
                sibling.close().await?;
                Ok::<(), anyhow::Error>(())
            },
            async |served| {
                served
                    .retire(
                        &options,
                        None,
                        std::time::Duration::from_secs(10),
                        "migrated safe-journal service owner did not reap",
                    )
                    .await
            },
        )
        .await;
        if let Err(error) = settled {
            return root.release(Err(error));
        }
        let store = super::super::MemoryStore::open(options.clone()).await?;
        let pending_legacy = store
            .public_transcript_page("legacy-session", None, 16)
            .await?;
        assert!(matches!(
            pending_legacy.pending.as_ref(),
            Some(record)
                if record.kind == super::super::PublicTurnKind::LegacyContinuation
                    && record.user_entry.is_none()
                    && record.continuation_of_node_id.is_none()
        ));
        store
            .checkpoint_session_turn(
                &format!("{scope}/transcript/legacy-session"),
                "legacy-session",
                &[kuru_core::Message::text("assistant", "legacy retry answer")],
                &[(
                    legacy_journal_key.clone(),
                    json!({"state": "ended after legacy retry"}),
                )],
                &super::super::SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: legacy_turn_id.into(),
                    settlement: super::super::PublicTurnSettlement::Completed,
                    speaker_id: "legacy-speaker".into(),
                },
            )
            .await?;
        let resumed_legacy = store
            .public_transcript_page("legacy-session", None, 16)
            .await?;
        assert!(resumed_legacy.pending.is_none());
        assert_eq!(resumed_legacy.records.len(), 2);
        assert!(resumed_legacy.records.iter().any(|entry| matches!(
            entry,
            super::super::PublicTranscriptEntry::Turn { record }
                if record.kind == super::super::PublicTurnKind::LegacyContinuation
                    && record.user_entry.is_none()
                    && record.terminal_entries
                        == [kuru_core::Message::text("assistant", "legacy retry answer")]
        )));
        assert_eq!(
            store
                .history(&format!("{scope}/transcript/legacy-session"), 16)
                .await?,
            [
                kuru_core::Message::text("user", "legacy visible transcript"),
                kuru_core::Message::text("assistant", "legacy retry answer")
            ]
        );
        let retained_ambiguous: String = bounded_query(
            sqlx::query_scalar("SELECT value FROM state WHERE BINARY `key` = BINARY ?")
                .bind(format!("{scope}/session/ambiguous-only").into_bytes())
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(retained_ambiguous, ambiguous_value);
        let retained_ambiguous_message: String = bounded_query(
            sqlx::query_scalar("SELECT content FROM messages WHERE BINARY namespace = BINARY ?")
                .bind(format!("{scope}/transcript/ambiguous-only").into_bytes())
                .fetch_one(store.pool.as_ref()),
        )
        .await?;
        assert_eq!(retained_ambiguous_message, "ambiguous transcript retained");
        type LegacyProvenanceColumns = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>, String);
        let row: LegacyProvenanceColumns = bounded_query(
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
        let ambiguous_key = format!("{scope}/session/ambiguous-only");
        let ambiguous_export_value: serde_json::Value = serde_json::from_str(ambiguous_value)?;
        assert!(records.iter().any(|record| matches!(record,
            super::super::StorageRecord::State { key, value }
                if key == &ambiguous_key && value == &ambiguous_export_value
        )));
        let ambiguous_namespace = format!("{scope}/transcript/ambiguous-only");
        assert!(records.iter().any(|record| matches!(record,
            super::super::StorageRecord::Message { namespace, session_id: None, role, content_format, content, .. }
                if namespace == &ambiguous_namespace
                    && role == "assistant"
                    && content_format == "text-v1"
                    && content == "ambiguous transcript retained"
        )));
        let before_refusal = durable_snapshot(&store.pool).await?;
        let exported_before_refusal = serde_json::to_value(&records)?;
        let error = validate_active_with(RELEASED_V7_REGISTRY, &store.pool)
            .await
            .expect_err("a v7 validator must refuse this populated v10 store");
        assert!(
            format!("{error:#}").contains("unsupported Dolt memory schema version 10"),
            "unexpected v7-validator refusal: {error:#}"
        );
        assert_eq!(durable_snapshot(&store.pool).await?, before_refusal);
        let after_refusal = store.begin_active_export().await?;
        let mut cursor = None;
        let mut exported_after_refusal = Vec::new();
        loop {
            let page = after_refusal.page(cursor).await?;
            exported_after_refusal.extend(page.records);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            serde_json::to_value(&exported_after_refusal)?,
            exported_before_refusal,
            "v6-validator refusal changed v7 application rows"
        );
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
