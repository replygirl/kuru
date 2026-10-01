//! Main-pool classification of retained migration attempts.
//!
//! `classify_historical_attempts_in` answers every check from the caller's
//! own pool: branch heads from `dolt_branches`, working sets from the
//! branch-qualified `dolt_status`, rows at a commit with `AS OF`, parents from
//! `dolt_commit_ancestors`, ancestry from `dolt_log`, and full schema
//! validation on one detached session switched with `USE` to each commit.
//! These tests compare it with the branch-pool classifier it replaced
//! (`classify_with_branch_pools_in`, kept under `cfg(test)` as the oracle) on
//! real retained branches, for root and `kuru_reader`, and prove that opening
//! an existing project no longer opens a pool on any migration branch or
//! commit.
use super::super::MemoryStore;
use super::*;
use crate::server::ServerOptions;
use serde::Serialize;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const SESSION_WAIT: Duration = Duration::from_secs(10);

type Verdict = std::result::Result<(), String>;

fn verdict(result: Result<()>) -> Verdict {
    result.map_err(|error| format!("{error:#}"))
}

#[derive(Clone, Copy, Debug)]
enum Expect {
    Pass,
    Fail(&'static str),
}

/// The branch-pool oracle's and the main-pool classifier's verdicts on the
/// same store, through the same role's server and pool.
async fn verdicts(
    registry: Registry,
    server: &Server,
    main: &MySqlPool,
    prefix: &str,
) -> (Verdict, Verdict) {
    let oracle = verdict(
        classify_with_branch_pools_in(registry, server, main, registry.current, prefix).await,
    );
    let requested = server.pool_requests();
    let main_pool =
        verdict(classify_historical_attempts_in(registry, main, registry.current, prefix).await);
    assert_eq!(
        server.pool_requests(),
        requested,
        "main-pool classification requested a pool"
    );
    (oracle, main_pool)
}

fn assert_expected(label: &str, role: &str, observed: &Verdict, expect: Expect) {
    match (expect, observed) {
        (Expect::Pass, Ok(())) => {}
        (Expect::Fail(needle), Err(message)) if message.contains(needle) => {}
        _ => panic!("{label} ({role}): expected {expect:?}, observed {observed:?}"),
    }
}

/// Both roles agree with the oracle, and the verdict is the state's own.
async fn assert_parity(
    label: &str,
    roles: &[(&str, &Server, &MySqlPool)],
    expect: Expect,
) -> Vec<String> {
    let mut rows = Vec::new();
    for (role, server, main) in roles {
        let (oracle, main_pool) = verdicts(REGISTRY, server, main, RESERVED_PREFIX).await;
        assert_eq!(
            main_pool, oracle,
            "{label} ({role}): main-pool verdict differs from the branch-pool oracle"
        );
        assert_expected(label, role, &main_pool, expect);
        rows.push(format!("{label} ({role}): {main_pool:?}"));
    }
    rows
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Ref {
    hash: String,
    dirty: bool,
}

async fn refs(main: &MySqlPool) -> Result<BTreeMap<String, Ref>> {
    let rows = bounded_query(
        sqlx::query("SELECT name, hash, dirty FROM dolt_branches ORDER BY name LIMIT 200")
            .fetch_all(main),
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("name")?,
                Ref {
                    hash: row.try_get("hash")?,
                    dirty: row.try_get::<bool, _>("dirty")?,
                },
            ))
        })
        .collect()
}

async fn status_rows(main: &MySqlPool, branch: &str) -> Result<Vec<(String, i64, String)>> {
    let rows = bounded_query(
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT table_name, staged, status FROM {} ORDER BY BINARY table_name, staged, BINARY status LIMIT 16",
            WorkingSet::Branch(branch).status_table()?
        )))
        .fetch_all(main),
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("table_name")?,
                row.try_get("staged")?,
                row.try_get("status")?,
            ))
        })
        .collect()
}

fn retained(refs: &BTreeMap<String, Ref>, target: i32) -> Result<(String, Ref)> {
    refs.iter()
        .find(|(name, _)| parse_attempt(name).is_ok_and(|(version, _)| version == target))
        .map(|(name, reference)| (name.clone(), reference.clone()))
        .with_context(|| format!("store did not retain its v{target} migration branch"))
}

/// Run `statements` on `branch`'s working set from one detached root session
/// of main, then wait until the engine has ended that session.
async fn on_branch(store: &MemoryStore, branch: &str, statements: &[String]) -> Result<()> {
    let mut connection = acquire(&store.pool).await?.detach();
    let written = async {
        bounded_query(
            connection.execute(sqlx::AssertSqlSafe(format!("USE `{DATABASE}/{branch}`"))),
        )
        .await?;
        for statement in statements {
            bounded_query(
                sqlx::query(sqlx::AssertSqlSafe(statement.clone())).fetch_all(&mut connection),
            )
            .await
            .with_context(|| format!("fixture statement on {branch}: {statement}"))?;
        }
        Ok(())
    }
    .await;
    let closed = bounded_query(connection.close()).await;
    after_cleanup(written, closed)?;
    quiesce(store, branch).await
}

/// Wait until no engine session (either role's) sits on `branch`, so a ref
/// can be moved or deleted after the oracle's branch pools closed.
async fn quiesce(store: &MemoryStore, branch: &str) -> Result<()> {
    store
        .shared
        .server
        .await_branch_sessions_end(&store.pool, branch, SESSION_WAIT)
        .await
}

async fn branch_call(store: &MemoryStore, arguments: &[&str]) -> Result<()> {
    let placeholders = vec!["?"; arguments.len()].join(", ");
    let sql = format!("CALL DOLT_BRANCH({placeholders})");
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for argument in arguments {
        query = query.bind(*argument);
    }
    bounded_query(query.fetch_all(store.pool.as_ref())).await?;
    Ok(())
}

async fn delete_branch(store: &MemoryStore, branch: &str) -> Result<()> {
    quiesce(store, branch).await?;
    branch_call(store, &["-D", branch]).await
}

fn commit_statement(message: &str) -> String {
    format!("CALL DOLT_COMMIT('-Am', '{message}', '--author', '{AUTHOR}')")
}

/// A branch-pool verdict of full schema validation at one commit, used only
/// to confirm which side of a fixture carries the defect.
async fn validates_at(server: &Server, commit: &str, expected: i32) -> Result<bool> {
    let pool = server.pool(commit).await?;
    let validated = validate_version_with(REGISTRY, &pool, expected).await;
    close_branch_pool(&pool).await?;
    Ok(validated.is_ok())
}

async fn open_reader(store: &MemoryStore) -> Result<(Server, Arc<MySqlPool>)> {
    let directory = store.shared.directory.clone();
    let data_dir = directory
        .parent()
        .and_then(Path::parent)
        .context("store directory has no data directory")?
        .to_owned();
    // Warmed before the spawn gate is held; see `crate::spawn_gate`.
    let binary = crate::test_support::warm_runtime_cache().await?;
    let server = {
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(ServerOptions {
            binary,
            directory,
            project_scope: store.shared.project_scope.clone(),
            supervisor: super::super::test_supervisor()?,
            timeout: Duration::from_secs(
                super::super::OpenOptions::new(Default::default(), String::new())
                    .config
                    .startup_timeout_secs,
            ),
            read_only: true,
            retained: None,
            lifecycle_root: cfg!(windows).then(|| data_dir.join("memory/lifecycles")),
        })
        .await?
    };
    let pool = server.pool("main").await?;
    Ok((server, pool))
}

/// Parity over every retained-branch state the brief names, for root and for
/// `kuru_reader`, each fixture built, confirmed, compared and then removed so
/// the next state starts from the unchanged baseline refs.
#[tokio::test]
async fn main_pool_classification_agrees_with_branch_pool_classification() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let (reader_server, reader_pool) = open_reader(&store).await?;
    let outcome = async {
        let root_server = &store.shared.server;
        let roles: [(&str, &Server, &MySqlPool); 2] = [
            ("root", root_server, store.pool.as_ref()),
            ("kuru_reader", &reader_server, reader_pool.as_ref()),
        ];
        let baseline = refs(&store.pool).await?;
        let main_head = revision(&store.pool).await?;
        let (v3, v3_ref) = retained(&baseline, 3)?;
        let (_, v5_ref) = retained(&baseline, 5)?;
        let (_, v7_ref) = retained(&baseline, 7)?;
        let v7_parent = sole_parent(&store.pool, &v7_ref.hash).await?;
        let mut rows = Vec::new();

        // Clean published branches, on main and on the usage ledger.
        rows.extend(assert_parity("clean published", &roles, Expect::Pass).await);
        let usage = root_server.pool(super::super::usage_ledger::BRANCH).await?;
        let (oracle, main_pool) =
            verdicts(USAGE_REGISTRY, root_server, &usage, USAGE_RESERVED_PREFIX).await;
        assert_eq!(main_pool, oracle, "usage ledger: verdicts differ");
        assert_expected("usage ledger", "root", &main_pool, Expect::Pass);
        rows.push(format!("usage ledger clean (root): {main_pool:?}"));

        // A published branch with a dirtied working set.
        on_branch(
            &store,
            &v3,
            &["CREATE TABLE p2_dirty_probe (id INT PRIMARY KEY)".to_owned()],
        )
        .await?;
        ensure!(refs(&store.pool).await?[&v3].dirty, "dirty fixture is clean");
        ensure!(
            status_rows(&store.pool, &v3).await?
                == [("p2_dirty_probe".to_owned(), 0, "new table".to_owned())],
            "dirty fixture has an unexpected working set"
        );
        rows.extend(
            assert_parity(
                "dirtied published working set",
                &roles,
                Expect::Fail("dirty historical Dolt migration branch has an unexpected schema"),
            )
            .await,
        );
        on_branch(
            &store,
            &v3,
            &[
                "DROP TABLE p2_dirty_probe".to_owned(),
                "CALL DOLT_RESET('--hard')".to_owned(),
            ],
        )
        .await?;
        assert_eq!(refs(&store.pool).await?, baseline, "dirty fixture not restored");

        // A retained failed attempt: exact base, the step's own failed
        // working set, and then an unexpected one.
        for (label, statements, expect) in [
            (
                "dirty failed attempt with its declared working set",
                V7.sql
                    .iter()
                    .map(|statement| (*statement).to_owned())
                    .collect::<Vec<_>>(),
                Expect::Pass,
            ),
            (
                "dirty failed attempt with an unexpected working set",
                vec!["CREATE TABLE p2_unrelated (id INT PRIMARY KEY)".to_owned()],
                Expect::Fail("failed-status inventory is incomplete or excessive"),
            ),
        ] {
            let name = attempt_name(7, Uuid::new_v4());
            branch_call(&store, &[&name, &v7_parent]).await?;
            on_branch(&store, &name, &statements).await?;
            ensure!(refs(&store.pool).await?[&name].dirty, "{label}: not dirty");
            rows.extend(assert_parity(label, &roles, expect).await);
            delete_branch(&store, &name).await?;
            assert_eq!(refs(&store.pool).await?, baseline, "{label}: not removed");
        }

        // A ref force-moved to another commit.
        quiesce(&store, &v3).await?;
        branch_call(&store, &["-f", &v3, &main_head]).await?;
        ensure!(
            refs(&store.pool).await?[&v3].hash == main_head,
            "moved-ref fixture did not move"
        );
        rows.extend(
            assert_parity(
                "ref force-moved to main",
                &roles,
                Expect::Fail("schema version 7, expected 3"),
            )
            .await,
        );
        quiesce(&store, &v3).await?;
        branch_call(&store, &["-f", &v3, &v3_ref.hash]).await?;
        assert_eq!(refs(&store.pool).await?, baseline, "moved ref not restored");

        // Receipts: a name whose UUID is not the receipt's, a removed row, and
        // a rewritten row that matches the name on a non-exact-base commit.
        let alias = attempt_name(3, Uuid::new_v4());
        branch_call(&store, &[&alias, &v3_ref.hash]).await?;
        rows.extend(
            assert_parity(
                "receipt does not match the branch name",
                &roles,
                Expect::Fail("receipt does not match its branch"),
            )
            .await,
        );
        delete_branch(&store, &alias).await?;
        let operation = Uuid::new_v4();
        for (label, statement, expect) in [
            (
                "receipt row removed",
                "DELETE FROM kuru_migrations WHERE version = 3".to_owned(),
                Expect::Fail("receipt chain is incomplete"),
            ),
            (
                "receipt row rewritten to the branch name",
                format!(
                    "UPDATE kuru_migrations SET operation = '{}' WHERE version = 3",
                    operation.hyphenated()
                ),
                Expect::Fail("schema version 3, expected 2"),
            ),
        ] {
            let name = attempt_name(3, operation);
            branch_call(&store, &[&name, &v3_ref.hash]).await?;
            on_branch(&store, &name, &[statement, commit_statement("P2 receipt fixture")])
                .await?;
            ensure!(
                refs(&store.pool).await?[&name].hash != v3_ref.hash,
                "{label}: fixture commit did not advance"
            );
            rows.extend(assert_parity(label, &roles, expect).await);
            delete_branch(&store, &name).await?;
        }
        assert_eq!(refs(&store.pool).await?, baseline, "receipt fixtures not removed");

        // Schema defects visible only through information_schema (a column
        // length), at the parent and then at the head of a v6 attempt.
        let narrow = "ALTER TABLE context_summaries MODIFY COLUMN summary_namespace VARBINARY(512) NOT NULL";
        let widen = "ALTER TABLE context_summaries MODIFY COLUMN summary_namespace VARBINARY(1024) NOT NULL";
        let step = |operation: Uuid| {
            let mut statements: Vec<String> =
                V6.sql.iter().map(|statement| (*statement).to_owned()).collect();
            statements.push("UPDATE kuru_schema SET version = 6 WHERE id = 1".to_owned());
            statements.push(format!(
                "INSERT INTO kuru_migrations (version, id, digest, operation) VALUES (6, '{}', '{}', '{}')",
                V6.id,
                digest(&V6),
                operation.hyphenated()
            ));
            statements
        };
        let wrong_base = "p2_wrong_parent_fixture";
        branch_call(&store, &[wrong_base, &v5_ref.hash]).await?;
        on_branch(
            &store,
            wrong_base,
            &[narrow.to_owned(), commit_statement("P2 wrong parent schema")],
        )
        .await?;
        let wrong_parent = refs(&store.pool).await?[wrong_base].hash.clone();
        let operation = Uuid::new_v4();
        let name = attempt_name(6, operation);
        branch_call(&store, &[&name, &wrong_parent]).await?;
        let mut statements = vec![widen.to_owned()];
        statements.extend(step(operation));
        statements.push(commit_statement("P2 attempt on a wrong parent"));
        on_branch(&store, &name, &statements).await?;
        let head = refs(&store.pool).await?[&name].hash.clone();
        ensure!(
            validates_at(root_server, &head, 6).await?
                && !validates_at(root_server, &wrong_parent, 5).await?,
            "wrong-parent fixture: the defect is not at the parent alone"
        );
        rows.extend(
            assert_parity(
                "parent with a wrong schema",
                &roles,
                Expect::Fail("context_summaries column summary_namespace differs"),
            )
            .await,
        );
        delete_branch(&store, &name).await?;
        delete_branch(&store, wrong_base).await?;

        let operation = Uuid::new_v4();
        let name = attempt_name(6, operation);
        branch_call(&store, &[&name, &v5_ref.hash]).await?;
        let mut statements = step(operation);
        statements.push(narrow.to_owned());
        statements.push(commit_statement("P2 attempt with a wrong head schema"));
        on_branch(&store, &name, &statements).await?;
        let head = refs(&store.pool).await?[&name].hash.clone();
        ensure!(
            !validates_at(root_server, &head, 6).await?
                && validates_at(root_server, &v5_ref.hash, 5).await?,
            "wrong-head fixture: the defect is not at the head alone"
        );
        rows.extend(
            assert_parity(
                "head with a wrong schema",
                &roles,
                Expect::Fail("context_summaries column summary_namespace differs"),
            )
            .await,
        );
        delete_branch(&store, &name).await?;
        assert_eq!(refs(&store.pool).await?, baseline, "schema fixtures not removed");
        rows.extend(assert_parity("clean after restore", &roles, Expect::Pass).await);
        eprintln!("P2 parity:\n{}", rows.join("\n"));
        Ok::<_, anyhow::Error>(())
    }
    .await;
    reader_pool.close().await;
    drop(reader_pool);
    let reader_closed = reader_server.close().await;
    let store_closed = store.close().await;
    after_cleanup(after_cleanup(outcome, reader_closed), store_closed)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRecord {
    version: u32,
    instance: String,
    project_scope: String,
    password: String,
    reader_password: String,
    initialized: bool,
}

/// Adopt main and the usage ledger to `instance` on one detached session, as
/// a template-born store will: retained migration branches keep the old row.
async fn adopt(main: &MySqlPool, scope: &str, from: &str, to: &str) -> Result<()> {
    let mut connection = acquire(main).await?.detach();
    let adopted = async {
        for database in [
            format!("{DATABASE}/{}", super::super::usage_ledger::BRANCH),
            DATABASE.to_owned(),
        ] {
            bounded_query(connection.execute(sqlx::AssertSqlSafe(format!("USE `{database}`"))))
                .await?;
            let updated = bounded_query(
                sqlx::query("UPDATE kuru_instance SET instance_id = ? WHERE singleton = 1 AND instance_id = ? AND project_scope = ?")
                    .bind(to)
                    .bind(from)
                    .bind(scope)
                    .execute(&mut connection),
            )
            .await?
            .rows_affected();
            ensure!(updated == 1, "adoption updated {updated} rows on {database}");
            bounded_query(
                sqlx::query("CALL DOLT_COMMIT('-am', 'Adopt P2 fixture identity', '--author', ?)")
                    .bind(AUTHOR)
                    .fetch_all(&mut connection),
            )
            .await?;
        }
        Ok(())
    }
    .await;
    let closed = bounded_query(connection.close()).await;
    after_cleanup(adopted, closed)
}

fn is_revision_pool(name: &str) -> bool {
    name.starts_with(RESERVED_PREFIX)
        || name.starts_with(USAGE_RESERVED_PREFIX)
        || commit_hash(name).is_ok()
}

/// A store whose retained branches carry a different instance row from main
/// (the shape a template-born store has after adoption). The branch-pool
/// classifier cannot classify it: every branch pool is refused by the
/// per-connection identity check before any query. The main-pool classifier
/// classifies the same branches by content and reaches the verdict the oracle
/// reached on the same branches before adoption. The divergence from the
/// oracle on the adopted store is the purpose of this change, not a parity
/// failure, and the test asserts it explicitly.
#[tokio::test]
async fn adopted_store_classifies_retained_branches_from_main() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let scope = format!("project/{}", "e".repeat(64));
    let mut options =
        crate::test_support::warmed_open_options(root.path().join("private"), scope.clone())
            .await?;
    // The branch-pool oracle needs retained branches that carry main's own
    // identity: a cold store, not a template copy.
    options.creation = crate::store::Creation::Cold;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    let (source_oracle, source_main_pool) =
        verdicts(REGISTRY, &store.shared.server, &store.pool, RESERVED_PREFIX).await;
    assert_eq!(source_oracle, Ok(()), "source oracle verdict");
    assert_eq!(source_main_pool, Ok(()), "source main-pool verdict");
    let branches: Vec<(String, String)> = refs(&store.pool)
        .await?
        .into_iter()
        .filter(|(name, _)| name.starts_with(RESERVED_PREFIX))
        .map(|(name, reference)| (name, reference.hash))
        .collect();
    ensure!(!branches.is_empty(), "no retained migration branch");
    let directory = store.shared.directory.clone();
    let identity_path = directory.join("identity.json");
    let identity: IdentityRecord =
        serde_json::from_slice(&crate::files::read_bytes(&identity_path, 16 * 1024)?)?;
    let instance = Uuid::new_v4().to_string();
    let adopted = adopt(&store.pool, &scope, &identity.instance, &instance).await;
    store.close().await?;
    adopted?;
    crate::files::write(
        &identity_path,
        &serde_json::to_vec(&IdentityRecord {
            instance: instance.clone(),
            ..identity
        })?,
    )?;

    // The ordinary writable open classifies every retained branch.
    let reopened = crate::test_support::spawn_gated_open(options).await?;
    let checked = async {
        let requests = reopened.shared.server.pool_requests();
        ensure!(
            !requests.iter().any(|name| is_revision_pool(name)),
            "adopted open pooled a migration branch or commit: {requests:?}"
        );
        for (name, head) in &branches {
            let row: String = bounded_query(
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT instance_id FROM kuru_instance AS OF '{}' WHERE singleton = 1",
                    commit_hash(head)?
                )))
                .fetch_one(reopened.pool.as_ref()),
            )
            .await?;
            ensure!(
                row != instance,
                "{name} carries main's adopted instance, not the old one"
            );
        }
        let (oracle, main_pool) = verdicts(
            REGISTRY,
            &reopened.shared.server,
            &reopened.pool,
            RESERVED_PREFIX,
        )
        .await;
        assert_eq!(
            main_pool, source_oracle,
            "adopted main-pool verdict differs from the oracle's on the same branches before adoption"
        );
        let refused = oracle.expect_err("branch pools classified an adopted store");
        ensure!(
            refused.contains("identity mismatch"),
            "branch-pool oracle failed for a reason other than identity: {refused}"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, reopened.close().await)
}

/// Opening an existing project, writable or read-only, pools no migration
/// branch or commit; the oracle on the same server does (a positive control
/// for the hook), and the validators called directly add nothing.
#[tokio::test]
async fn historical_classification_opens_no_branch_or_commit_pools() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::warmed_open_options(
        root.path().join("private"),
        format!("project/{}", "d".repeat(64)),
    )
    .await?;
    // The branch-pool oracle below needs a cold store's retained branches.
    options.creation = crate::store::Creation::Cold;
    crate::test_support::spawn_gated_open(options.clone())
        .await?
        .close()
        .await?;

    let reopened = crate::test_support::spawn_gated_open(options.clone()).await?;
    let checked = async {
        let server = &reopened.shared.server;
        let opened = server.pool_requests();
        ensure!(
            !opened.iter().any(|name| is_revision_pool(name)),
            "writable open pooled a migration branch or commit: {opened:?}"
        );
        validate_active(&reopened.pool).await?;
        validate_ready(&reopened.pool).await?;
        ensure!(
            server.pool_requests() == opened,
            "direct validation requested a pool"
        );
        let retained = reserved_names(&reopened.pool).await?.len();
        ensure!(retained > 0, "no retained migration branch");
        classify_with_branch_pools_in(
            REGISTRY,
            server,
            &reopened.pool,
            REGISTRY.current,
            RESERVED_PREFIX,
        )
        .await?;
        let oracle = server.pool_requests()[opened.len()..].to_vec();
        ensure!(
            oracle.len() == 2 * retained && oracle.iter().all(|name| is_revision_pool(name)),
            "oracle pools per clean retained branch: {oracle:?} for {retained} branches"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, reopened.close().await)?;

    options.read_only = true;
    let inspected = crate::test_support::spawn_gated_open(options).await?;
    let requests = inspected.shared.server.pool_requests();
    let checked = if requests.iter().any(|name| is_revision_pool(name)) {
        Err(anyhow::anyhow!(
            "read-only open pooled a migration branch or commit: {requests:?}"
        ))
    } else {
        validate_inspection(&inspected.pool).await
    };
    after_cleanup(checked, inspected.close().await)
}

/// A pool whose every acquire fails: any statement sent to it surfaces as a
/// connection error, never as the input guard's message.
fn unreachable_pool() -> MySqlPool {
    MySqlPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy_with(
            MySqlConnectOptions::new()
                .host("127.0.0.1")
                .port(1)
                .username("nobody")
                .database(DATABASE)
                .ssl_mode(MySqlSslMode::Disabled),
        )
}

#[tokio::test]
async fn malformed_branch_hash_never_reaches_revision_sql() -> Result<()> {
    let valid = "0123456789abcdefghijklmnopqrstuv";
    assert_eq!(commit_hash(valid)?, valid);
    for malformed in [
        "",
        "invalid' revision",
        "0123456789abcdefghijklmnopqrstu",
        "0123456789abcdefghijklmnopqrstuvw",
        "0123456789ABCDEFGHIJKLMNOPQRSTUV",
        "0123456789abcdefghijklmnopqrstuw",
        "0123456789abcdefghijklmnopqrst`v",
    ] {
        assert!(commit_hash(malformed).is_err(), "{malformed:?} accepted");
    }
    let pool = unreachable_pool();
    let guard = "not a commit hash";
    let malformed = "invalid' revision";
    for (label, result) in [
        (
            "version AS OF",
            version_as_of(&pool, malformed).await.map(|_| ()),
        ),
        (
            "receipt AS OF",
            receipt_as_of(&pool, malformed, 2).await.map(|_| ()),
        ),
    ] {
        let error = format!("{:#}", result.expect_err(label));
        assert!(error.contains(guard), "{label} reached the engine: {error}");
    }
    let mut revisions = RevisionReader::default();
    let error = format!(
        "{:#}",
        revisions
            .validate(&pool, REGISTRY, malformed, 2)
            .await
            .expect_err("USE accepted a malformed revision")
    );
    assert!(error.contains(guard), "USE reached the engine: {error}");
    assert!(
        revisions.connection.is_none(),
        "a malformed revision acquired a session"
    );
    revisions.close().await?;
    for name in ["kuru_migration_v`x", "kuru/main", "a b", ""] {
        assert!(
            WorkingSet::Branch(name).status_table().is_err(),
            "{name:?} accepted as a branch working set"
        );
    }
    // The guard is what failed: a well-formed revision does reach the pool.
    let reached = format!(
        "{:#}",
        version_as_of(&pool, valid)
            .await
            .expect_err("unreachable pool answered")
    );
    assert!(
        !reached.contains(guard),
        "valid hash was refused: {reached}"
    );
    pool.close().await;
    Ok(())
}

/// Each main-pool read refuses a missing or unexpected row instead of
/// treating the branch as classified.
#[tokio::test]
async fn main_pool_reads_fail_closed_on_missing_rows() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let checked = async {
        let main = store.pool.as_ref();
        let absent = attempt_name(2, Uuid::new_v4());
        let error = format!(
            "{:#}",
            branch_head(main, &absent)
                .await
                .expect_err("absent branch had a head")
        );
        assert!(error.contains("missing or ambiguous"), "{error}");
        ensure!(
            working_set_changes(main, WorkingSet::Branch(&absent))
                .await
                .is_err(),
            "absent branch had a working set"
        );
        let missing = "0000000000000000000000000000000v";
        ensure!(
            version_as_of(main, missing).await.is_err(),
            "absent commit had a schema version"
        );
        let refs = refs(main).await?;
        let (_, v2) = retained(&refs, 2)?;
        ensure!(
            receipt_as_of(main, &v2.hash, 99).await.is_err(),
            "absent receipt row was read"
        );
        let mut revisions = RevisionReader::default();
        let validated = revisions.validate(main, REGISTRY, missing, 2).await;
        revisions.close().await?;
        ensure!(validated.is_err(), "absent commit validated");
        let mut revisions = RevisionReader::default();
        let validated = revisions.validate(main, REGISTRY, &v2.hash, 3).await;
        revisions.close().await?;
        let error = format!("{:#}", validated.expect_err("v2 commit validated as v3"));
        assert!(error.contains("schema version 2, expected 3"), "{error}");
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// In-process cost of `validate_active` on a v7 store with its six retained
/// branches, against the replaced branch-pool path reconstructed from the
/// oracle (`validate_current_with`, `clean`, `classify_with_branch_pools_in`).
/// Report only; run explicitly:
/// `cargo test -p kuru-memory --lib --all-features measure_validate_active -- --ignored --nocapture`
#[tokio::test]
#[ignore = "measurement, run explicitly with --ignored --nocapture"]
async fn measure_validate_active_classification() -> Result<()> {
    let iterations: usize = std::env::var("KURU_TEST_P2_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let store = MemoryStore::temporary_cold().await?;
    let measured = async {
        let server = &store.shared.server;
        let main = store.pool.as_ref();
        let branch_pools = || async {
            validate_current_with(REGISTRY, main).await?;
            clean(main).await?;
            classify_with_branch_pools_in(REGISTRY, server, main, REGISTRY.current, RESERVED_PREFIX)
                .await
        };
        // One unmeasured warm-up of each.
        validate_active(main).await?;
        branch_pools().await?;
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for _ in 0..iterations {
            let started = Instant::now();
            branch_pools().await?;
            before.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            validate_active(main).await?;
            after.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        eprintln!(
            "P2 validate_active ms, {} retained branches, N={iterations}: branch pools {before:.1?} (median {:.1}); main pool {after:.1?} (median {:.1})",
            reserved_names(main).await?.len(),
            median(&mut before.clone()),
            median(&mut after.clone()),
        );
        Ok(())
    }
    .await;
    after_cleanup(measured, store.close().await)
}
