//! Migration publication records (schema 8) against the real engine.
//!
//! A main step at or above [`PUBLICATION_VERSION`] records its own branch,
//! exact base, receipt operation and definition digest inside its attempt
//! commit; the step that introduces records also records every branch the
//! same open's full classification accepted as published. A later open
//! verifies a recorded branch by its record from main's pool and classifies
//! every other retained branch in full, as before. These fixtures build the
//! released v7 shape directly, inspect the attempt commit before and after its
//! fast-forward, count by-record and full classifications, and break each
//! axis a record is checked on.
use super::super::MemoryStore;
use super::tests::{
    DurableSnapshot, RELEASED_V7_REGISTRY, RELEASED_V8_REGISTRY, TEST_REGISTRY,
    assert_attempts_unchanged, assert_failed_runner_unchanged, durable_snapshot, snapshot_attempts,
};
use super::*;
use crate::store::OpenOptions;
use crate::test_support::engine_ledger;
use std::{
    path::Path,
    time::{Duration, Instant},
};

const TEST_DEADLINE: Duration = Duration::from_secs(60);

/// Engine starts this process made on store directories first started
/// beneath `root`.
fn starts_under(root: &Path) -> Result<u64> {
    let canonical = std::fs::canonicalize(root)?;
    Ok(engine_ledger::with(|ledger| {
        ledger.starts_under(&canonical)
    }))
}

/// A stopped store exactly as a v7 release left it: every v2..v7 step ran
/// through the ordinary migration path, so each left its retained branch,
/// and no publication record exists.
async fn released_v7(root: &Path, scope: char) -> Result<OpenOptions> {
    let options = crate::test_support::warmed_open_options(
        root.join("private"),
        format!("project/{}", scope.to_string().repeat(64)),
    )
    .await?;
    super::super::tests::released_v1(&options).await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let upgraded = async {
        upgrade_with(
            RELEASED_V7_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        ensure!(version(&main).await? == 7, "fixture is not at schema 7");
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(upgraded, server.close().await)?;
    Ok(options)
}

async fn publication_table_exists(pool: &MemoryPool) -> Result<bool> {
    let tables: i64 = bounded_query(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND BINARY table_name = BINARY 'kuru_migration_publications'",
        )
        .fetch_one(pool),
    )
    .await?;
    Ok(tables == 1)
}

/// The retained main attempts by target: name and head.
async fn retained(pool: &MemoryPool) -> Result<BTreeMap<i32, Vec<(String, String)>>> {
    let mut retained = BTreeMap::<i32, Vec<(String, String)>>::new();
    for reference in reserved_refs_in(pool, RESERVED_PREFIX).await? {
        let (target, _) = parse_attempt(&reference.name)?;
        retained
            .entry(target)
            .or_default()
            .push((reference.name, reference.hash));
    }
    Ok(retained)
}

/// The sole retained branch for `target`.
async fn sole_branch(pool: &MemoryPool, target: i32) -> Result<(String, String)> {
    let mut branches = retained(pool).await?.remove(&target).unwrap_or_default();
    ensure!(
        branches.len() == 1,
        "store retains {} branches for schema {target}",
        branches.len()
    );
    Ok(branches.remove(0))
}

/// Run `statements` on `branch`'s working set from one detached session of
/// `pool`, then close it.
async fn on_branch(pool: &MemoryPool, branch: &str, statements: &[String]) -> Result<()> {
    let mut connection = acquire(pool).await?.detach();
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
    after_cleanup(written, bounded_query(connection.close()).await)
}

async fn branch_call(pool: &MemoryPool, arguments: &[&str]) -> Result<()> {
    let mut query = sqlx::query(sqlx::AssertSqlSafe(format!(
        "CALL DOLT_BRANCH({})",
        placeholders(arguments.len())
    )));
    for argument in arguments {
        query = query.bind(*argument);
    }
    bounded_query(query.fetch_all(pool)).await?;
    Ok(())
}

/// Wait until no engine session sits on `branch`, so its ref can move.
async fn quiesce(store: &MemoryStore, branch: &str) -> Result<()> {
    store
        .shared
        .server
        .await_branch_sessions_end(&store.pool, branch, Duration::from_secs(10))
        .await
}

/// A record-aware classification of the current main: its counts, or the
/// refusal text.
async fn classify(pool: &MemoryPool) -> std::result::Result<Classification, String> {
    classify_historical_attempts_in(REGISTRY, pool, REGISTRY.current, RESERVED_PREFIX)
        .await
        .map_err(|error| format!("{error:#}"))
}

/// The record each published branch carries, from the refs alone: every
/// retained clean branch, with its head's parent as the base.
async fn expected_records(pool: &MemoryPool, published: &[Published]) -> Result<Vec<Record>> {
    let mut records = published
        .iter()
        .map(|published| Record::of(REGISTRY, published))
        .collect::<Result<Vec<_>>>()?;
    records.sort();
    ensure!(
        records_in(pool).await? == records,
        "main's publication records differ from the published branches"
    );
    Ok(records)
}

/// Task 2.2/2.3: the step's record and the backfill sit in the attempt commit
/// itself (one commit whose parent is the exact base and which carries the
/// v8 schema, receipt and records) while main is still at schema 7 without
/// the table; they reach main only with the fast-forward.
#[tokio::test]
async fn publication_record_is_in_the_attempt_commit_and_reaches_main_with_the_fast_forward()
-> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = released_v7(root.path(), 'a').await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let checked = async {
        let base = revision(&main).await?;
        let classified =
            classify_historical_attempts_in(REGISTRY, &main, 7, RESERVED_PREFIX).await?;
        ensure!(
            classified
                .published
                .iter()
                .map(|p| p.version)
                .collect::<Vec<_>>()
                == (2..=7).collect::<Vec<_>>(),
            "v7 store did not publish v2..v7: {classified:?}"
        );
        let (hooks, control) = MigrationRunnerHooks::paused(MigrationBoundary::BeforePublish);
        let (migrating_server, migrating_main) = (server.clone(), main.clone());
        let upgrading = tokio::spawn(async move {
            upgrade_with(
                RELEASED_V8_REGISTRY,
                &migrating_server,
                &migrating_main,
                &hooks,
            )
            .await
        });
        let _abort = AbortOnDrop(upgrading.abort_handle());
        tokio::time::timeout(TEST_DEADLINE, control.reached())
            .await
            .context("v8 upgrade did not reach its publication boundary")??;
        let branch = control.branch_name()?;
        let target = control.publication_target()?;
        let (to, operation) = parse_attempt(&branch)?;
        ensure!(to == 8, "the paused step is {branch}");

        // Before the fast-forward: main is unchanged and has no table.
        ensure!(revision(&main).await? == base, "main moved before publish");
        ensure!(version(&main).await? == 7, "main advanced before publish");
        ensure!(
            !publication_table_exists(&main).await?,
            "main holds publication records before publish"
        );
        // The attempt's head is one clean commit on the exact base that
        // carries schema 8, the receipt and every record.
        ensure!(
            working_set_changes(&main, WorkingSet::Branch(&branch)).await? == 0,
            "the attempt's records are not committed"
        );
        ensure!(sole_parent(&main, &target).await? == base, "attempt parent");
        ensure!(version_as_of(&main, &target).await? == 8, "attempt schema");
        ensure!(
            receipt_as_of(&main, &target, 8).await? == operation.hyphenated().to_string(),
            "attempt receipt"
        );
        let mut published = classified.published.clone();
        published.push(Published {
            version: 8,
            branch: branch.clone(),
            base: base.clone(),
            operation,
        });
        let mut expected = published
            .iter()
            .map(|published| Record::of(REGISTRY, published))
            .collect::<Result<Vec<_>>>()?;
        expected.sort();
        ensure!(
            records_as_of(&main, &target).await? == expected,
            "the attempt commit's records differ: {:?}",
            records_as_of(&main, &target).await?
        );
        // The base is the validated schema-7 head, which holds no table. (A
        // failing `AS OF` read here would leave its pooled main session on a
        // stale transaction, which publish's own head read could then reuse.)
        ensure!(version_as_of(&main, &base).await? == 7, "base schema");

        control.resume();
        tokio::time::timeout(TEST_DEADLINE, upgrading)
            .await
            .context("v8 upgrade did not settle")???;
        // After it: main is the attempt commit, records and all.
        ensure!(revision(&main).await? == target, "main is not the target");
        ensure!(
            records_in(&main).await? == expected,
            "main's records differ"
        );
        validate_active_with(RELEASED_V8_REGISTRY, &main).await?;
        eprintln!(
            "P6 attempt commit {target} on base {base}: records {:?}",
            expected
                .iter()
                .map(|record| record.version)
                .collect::<Vec<_>>()
        );
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(checked, server.close().await)
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Task 3.2: the v8 step records exactly the branches its own open's full
/// classification accepted (v2..v7), and never a retained dirty attempt; the
/// next classification accepts those by record and classifies only the dirty
/// attempt in full.
#[tokio::test]
async fn v8_backfills_every_published_branch_from_classification() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = released_v7(root.path(), 'b').await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let checked = async {
        // A retained failed v7 attempt: exact v6 base, V7's declared working
        // set, never committed.
        let (_, v7_head) = sole_branch(&main, 7).await?;
        let v6_head = sole_parent(&main, &v7_head).await?;
        let failed = attempt_name(7, Uuid::new_v4());
        branch_call(&main, &[&failed, &v6_head]).await?;
        on_branch(
            &main,
            &failed,
            &V7.sql
                .iter()
                .map(|statement| (*statement).to_owned())
                .collect::<Vec<_>>(),
        )
        .await?;
        let before = classify_historical_attempts_in(REGISTRY, &main, 7, RESERVED_PREFIX).await?;
        ensure!(
            before.by_record == 0 && before.full == 7,
            "a v7 store classified {before:?}"
        );

        upgrade_with(
            RELEASED_V8_REGISTRY,
            &server,
            &main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        let after =
            classify_historical_attempts_in(RELEASED_V8_REGISTRY, &main, 8, RESERVED_PREFIX)
                .await?;
        ensure!(
            after.by_record == 7 && after.full == 1,
            "the upgraded store classified {after:?}"
        );
        let records = expected_records(&main, &after.published).await?;
        ensure!(
            records
                .iter()
                .map(|record| record.version)
                .collect::<Vec<_>>()
                == (2..=8).collect::<Vec<_>>(),
            "records are not one per published step: {records:?}"
        );
        ensure!(
            records[..6]
                .iter()
                .zip(&before.published)
                .all(|(record, published)| record.branch == published.branch
                    && record.base == published.base),
            "the backfill differs from the classification it ran"
        );
        ensure!(
            records.iter().all(|record| record.branch != failed),
            "the dirty attempt was recorded"
        );
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(checked, server.close().await)
}

/// Task 6.2: a read-only open of a v7 store is refused with the existing
/// message, without migrating or mutating it.
#[tokio::test]
async fn read_only_open_of_v7_store_requires_writable_upgrade() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = released_v7(root.path(), 'c').await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let before = durable_snapshot(&main).await;
    main.close().await;
    drop(main);
    let before = after_cleanup(before, server.close().await)?;
    options.read_only = true;
    let error = match crate::test_support::spawn_gated_open(options.clone()).await {
        Ok(store) => {
            store.close().await?;
            bail!("a read-only open served a v7 store");
        }
        Err(error) => format!("{error:#}"),
    };
    ensure!(
        error.contains("memory schema version 7 requires writable upgrade to 10"),
        "unexpected read-only refusal: {error}"
    );
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let after = async {
        ensure!(version(&main).await? == 7, "the refused open migrated");
        ensure!(
            durable_snapshot(&main).await? == before,
            "the refused open changed the store"
        );
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(after, server.close().await)
}

/// Task 6.1: the first writable open of a v7 store migrates in its worker and
/// reopens (two engine starts), classifying in full one last time; the next
/// open accepts every published branch by record with no full
/// classification and pools no migration branch or commit.
#[tokio::test]
async fn v7_store_upgrades_once_with_two_starts_then_uses_records() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = released_v7(root.path(), 'd').await?;
    let starts = starts_under(root.path())?;
    let started = Instant::now();
    let upgraded = crate::test_support::spawn_gated_open(options.clone()).await?;
    let elapsed = started.elapsed();
    let checked = async {
        let made = starts_under(root.path())? - starts;
        ensure!(made == 2, "the upgrading open made {made} engine starts");
        ensure!(
            version(&upgraded.pool).await? == CURRENT_VERSION,
            "the open did not upgrade"
        );
        let classified = classify(&upgraded.pool).await.map_err(anyhow::Error::msg)?;
        expected_records(&upgraded.pool, &classified.published).await?;
        eprintln!("P6 v7 upgrade open: {made} starts, {elapsed:.2?}");
        Ok(())
    }
    .await;
    after_cleanup(checked, upgraded.close().await)?;

    let starts = starts_under(root.path())?;
    let reopened = crate::test_support::spawn_gated_open(options).await?;
    let checked = async {
        let made = starts_under(root.path())? - starts;
        ensure!(made == 1, "the reopen made {made} engine starts");
        let requests = reopened.shared.server.pool_requests();
        ensure!(
            !requests.iter().any(|name| name.starts_with(RESERVED_PREFIX)
                || name.starts_with(USAGE_RESERVED_PREFIX)
                || commit_hash(name).is_ok()),
            "the reopen pooled a migration branch or commit: {requests:?}"
        );
        let classified = classify(&reopened.pool).await.map_err(anyhow::Error::msg)?;
        ensure!(
            classified.full == 0 && classified.by_record == usize::try_from(CURRENT_VERSION - 1)?,
            "the reopened store classified {classified:?}"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, reopened.close().await)
}

/// Task 6.3: a ready v7 stage left by the previous binary is validated at its
/// own version by full classification (it has no records), and is then
/// upgraded through the ordinary path.
#[tokio::test]
async fn ready_v7_stage_from_previous_binary_is_classified_then_upgraded() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = released_v7(root.path(), 'e').await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let checked = async {
        ensure!(
            validate_ready(&main).await? == 7,
            "the stage is not ready at 7"
        );
        let classified =
            classify_historical_attempts_in(REGISTRY, &main, 7, RESERVED_PREFIX).await?;
        ensure!(
            classified.full == 6 && classified.by_record == 0,
            "a ready v7 stage classified {classified:?}"
        );
        upgrade(&server, &main).await?;
        validate_active(&main).await?;
        let classified = classify(&main).await.map_err(anyhow::Error::msg)?;
        ensure!(
            classified.full == 0 && classified.by_record == usize::try_from(CURRENT_VERSION - 1)?,
            "the upgraded stage classified {classified:?}"
        );
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(checked, server.close().await)
}

/// Task 4.5 and the verification by record on an ordinary current store: a
/// cold store's every published branch is accepted by record; a retained
/// failed attempt without a record is still classified in full and accepted
/// or refused exactly as before.
#[tokio::test]
async fn unrecorded_failed_attempt_is_still_fully_classified() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let checked = async {
        let main = store.pool.as_ref();
        let clean = classify(main).await.map_err(anyhow::Error::msg)?;
        ensure!(
            clean.full == 0 && clean.by_record == usize::try_from(CURRENT_VERSION - 1)?,
            "a cold store classified {clean:?}"
        );
        expected_records(main, &clean.published).await?;
        let base = revision(main).await?;
        let (_, v8_head) = sole_branch(main, 8).await?;
        let v7_head = sole_parent(main, &v8_head).await?;
        for (label, statements, accepted) in [
            (
                "declared failed working set",
                V8.sql
                    .iter()
                    .map(|statement| (*statement).to_owned())
                    .collect::<Vec<_>>(),
                true,
            ),
            (
                "unexpected failed working set",
                vec!["CREATE TABLE p6_unrelated (id INT PRIMARY KEY)".to_owned()],
                false,
            ),
        ] {
            let failed = attempt_name(8, Uuid::new_v4());
            branch_call(main, &[&failed, &v7_head]).await?;
            on_branch(main, &failed, &statements).await?;
            let verdict = classify(main).await;
            match (accepted, &verdict) {
                (true, Ok(classified)) => ensure!(
                    classified.full == 1
                        && classified.by_record == usize::try_from(CURRENT_VERSION - 1)?,
                    "{label}: classified {classified:?}"
                ),
                (false, Err(message)) => ensure!(
                    message.contains("failed-status inventory"),
                    "{label}: refused for another reason: {message}"
                ),
                _ => bail!("{label}: unexpected verdict {verdict:?}"),
            }
            quiesce(&store, &failed).await?;
            branch_call(main, &["-D", &failed]).await?;
        }
        ensure!(revision(main).await? == base, "classification moved main");
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// A recorded branch whose ref was deleted is tolerated while its base stays
/// in main's history (today's classification iterates existing refs only),
/// and refused once the record's base leaves it.
#[tokio::test]
async fn recorded_branch_deleted_is_tolerated() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let checked = async {
        let main = store.pool.as_ref();
        let (v3, _) = sole_branch(main, 3).await?;
        quiesce(&store, &v3).await?;
        branch_call(main, &["-D", &v3]).await?;
        let classified = classify(main).await.map_err(anyhow::Error::msg)?;
        ensure!(
            classified.by_record == usize::try_from(CURRENT_VERSION - 2)? && classified.full == 0,
            "classified {classified:?} with a deleted recorded branch"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// The kinds of disagreement between a record and the store it is read from.
#[derive(Clone, Copy, Debug)]
enum Mismatch {
    /// The recorded ref force-moved to another commit of main's history.
    MovedRef,
    /// The recorded branch's working set changed.
    DirtyBranch,
    /// The record's base is a commit outside main's history.
    OutsideHistory,
    /// Head and recorded base agree with main's history, but the head does not
    /// carry the recorded step.
    AsOfReceipt,
    /// The record names another definition.
    Digest,
    /// The record names an operation main's receipt does not.
    Operation,
    /// The record has another format.
    Format,
}

/// Task 4.3: each kind of disagreement fails classification closed, names
/// the record, mutates nothing and never falls back to full classification
/// (which would accept the same store with records ignored where the refs
/// themselves are sound).
#[tokio::test]
async fn record_mismatches_fail_closed_without_mutation_or_fallback() -> Result<()> {
    for mismatch in [
        Mismatch::MovedRef,
        Mismatch::DirtyBranch,
        Mismatch::OutsideHistory,
        Mismatch::AsOfReceipt,
        Mismatch::Digest,
        Mismatch::Operation,
        Mismatch::Format,
    ] {
        let store = MemoryStore::temporary_cold().await?;
        let checked = async {
            let main = store.pool.as_ref();
            let (v3, v3_head) = sole_branch(main, 3).await?;
            let (v4, v4_head) = sole_branch(main, 4).await?;
            let v2_head = sole_parent(main, &v3_head).await?;
            let main_head = revision(main).await?;
            let edit_record = |set: &str, version: i32| {
                vec![
                    format!("UPDATE kuru_migration_publications SET {set} WHERE version = {version}"),
                    "CALL DOLT_COMMIT('-am', 'P6 record fixture', '--author', 'Kuru <memory@kuru.local>')"
                        .to_owned(),
                ]
            };
            let (expected, sound_refs) = match mismatch {
                Mismatch::MovedRef => {
                    quiesce(&store, &v3).await?;
                    branch_call(main, &["-f", &v3, &v4_head]).await?;
                    ("not the sole child of its recorded base", false)
                }
                Mismatch::DirtyBranch => {
                    on_branch(
                        main,
                        &v3,
                        &["CREATE TABLE p6_dirty_probe (id INT PRIMARY KEY)".to_owned()],
                    )
                    .await?;
                    ("published migration branch has uncommitted changes", false)
                }
                Mismatch::OutsideHistory => {
                    // A side commit on another branch: a real commit that main's
                    // history does not hold.
                    let side = "p6_side_history";
                    branch_call(main, &[side, &main_head]).await?;
                    on_branch(
                        main,
                        side,
                        &[
                            "INSERT INTO state (`key`, value) VALUES ('p6-side', '{}')".to_owned(),
                            "CALL DOLT_COMMIT('-am', 'P6 side commit', '--author', 'Kuru <memory@kuru.local>')"
                                .to_owned(),
                        ],
                    )
                    .await?;
                    let side_head = branch_head(main, side).await?;
                    quiesce(&store, &v3).await?;
                    branch_call(main, &["-D", &v3]).await?;
                    on_branch(main, "main", &edit_record(&format!("base = '{side_head}'"), 3))
                        .await?;
                    ("outside main's history", true)
                }
                Mismatch::AsOfReceipt => {
                    // The v4 ref moved to the v3 commit and its record's base to
                    // v3's parent: ref, parent and ancestry agree, content does not.
                    quiesce(&store, &v4).await?;
                    branch_call(main, &["-f", &v4, &v3_head]).await?;
                    on_branch(main, "main", &edit_record(&format!("base = '{v2_head}'"), 4))
                        .await?;
                    ("does not carry its recorded schema and receipt", false)
                }
                Mismatch::Digest => {
                    on_branch(
                        main,
                        "main",
                        &edit_record(&format!("definition_digest = '{}'", "0".repeat(64)), 3),
                    )
                    .await?;
                    ("names another definition", true)
                }
                Mismatch::Operation => {
                    on_branch(
                        main,
                        "main",
                        &edit_record(
                            &format!("operation = '{}'", Uuid::new_v4().hyphenated()),
                            3,
                        ),
                    )
                    .await?;
                    ("disagrees with its branch name", true)
                }
                Mismatch::Format => {
                    on_branch(main, "main", &edit_record("record_format = 2", 3)).await?;
                    ("has format 2", true)
                }
            };
            let before = durable_snapshot(main).await?;
            let records_before = records_in(main).await?;
            let refused = classify(main)
                .await
                .expect_err("a record mismatch was accepted");
            ensure!(
                refused.contains(expected),
                "{mismatch:?}: refused for another reason: {refused}"
            );
            ensure!(
                durable_snapshot(main).await? == before && records_in(main).await? == records_before,
                "{mismatch:?}: classification mutated the store"
            );
            // No fallback: the same store with records ignored reaches full
            // classification's own verdict, which accepts it where the refs
            // are sound.
            let full = classify_historical_attempts_with(
                REGISTRY,
                main,
                REGISTRY.current,
                RESERVED_PREFIX,
                Records::Ignore,
            )
            .await;
            ensure!(
                full.is_ok() == sound_refs,
                "{mismatch:?}: full classification verdict {:?}",
                full.map(|_| ())
            );
            // The open itself fails closed through the same validator.
            let refused = validate_active(main)
                .await
                .expect_err("validate_active accepted a record mismatch");
            ensure!(
                format!("{refused:#}").contains(expected),
                "{mismatch:?}: validate_active refused for another reason: {refused:#}"
            );
            eprintln!("P6 {mismatch:?}: refused ({expected})");
            Ok::<_, anyhow::Error>(())
        }
        .await;
        after_cleanup(checked, store.close().await)?;
    }
    Ok(())
}

/// The open refuses a store whose recorded branch is dirty, and the refusal
/// leaves the store as it was.
#[tokio::test]
async fn dirty_recorded_branch_fails_the_open_closed() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::warmed_open_options(
        root.path().join("private"),
        format!("project/{}", "f".repeat(64)),
    )
    .await?;
    options.creation = crate::store::Creation::Cold;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    let prepared = async {
        let (v5, _) = sole_branch(&store.pool, 5).await?;
        on_branch(
            &store.pool,
            &v5,
            &["CREATE TABLE p6_dirty_probe (id INT PRIMARY KEY)".to_owned()],
        )
        .await?;
        durable_snapshot(&store.pool).await
    }
    .await;
    let before = after_cleanup(prepared, store.close().await)?;
    let error = match crate::test_support::spawn_gated_open(options.clone()).await {
        Ok(store) => {
            store.close().await?;
            bail!("an open accepted a dirty recorded branch");
        }
        Err(error) => format!("{error:#}"),
    };
    ensure!(
        error.contains("published migration branch has uncommitted changes"),
        "unexpected open refusal: {error}"
    );
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let after = durable_snapshot(&main).await;
    main.close().await;
    drop(main);
    let after = after_cleanup(after, server.close().await)?;
    ensure!(after == before, "the refused open changed the store");
    Ok(())
}

/// Task 4.2: a record field that is not a well-formed hash or reserved name
/// is refused before any of it reaches SQL text.
#[tokio::test]
async fn malformed_record_values_never_reach_sql_text() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let checked = async {
        let main = store.pool.as_ref();
        for (label, set, expected) in [
            (
                "base",
                "base = 'abc'' OR ''1''=''1'",
                "names a malformed base",
            ),
            ("branch", "branch = 'kuru_migration_v`x'", "names a malformed branch"),
        ] {
            let head = revision(main).await?;
            on_branch(
                main,
                "main",
                &[
                    format!("UPDATE kuru_migration_publications SET {set} WHERE version = 3"),
                    "CALL DOLT_COMMIT('-am', 'P6 malformed record', '--author', 'Kuru <memory@kuru.local>')"
                        .to_owned(),
                ],
            )
            .await?;
            let refused = classify(main)
                .await
                .expect_err("a malformed record was accepted");
            ensure!(
                refused.contains(expected),
                "{label}: refused for another reason: {refused}"
            );
            on_branch(
                main,
                "main",
                &[format!("CALL DOLT_RESET('--hard', '{}')", commit_hash(&head)?)],
            )
            .await?;
        }
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// Task 5.1: a working change to the publication table is a schema
/// authority violation, as a change to the schema or receipts is.
#[tokio::test]
async fn publication_table_change_is_a_schema_authority_violation() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let checked = async {
        let main = store.pool.as_ref();
        bounded_query(
            sqlx::query(
                "UPDATE kuru_migration_publications SET record_format = 2 WHERE version = 2",
            )
            .execute(main),
        )
        .await?;
        let refused = validate_inspection(main)
            .await
            .expect_err("an uncommitted record change was accepted");
        ensure!(
            format!("{refused:#}").contains("migration receipt authority has uncommitted changes"),
            "unexpected refusal: {refused:#}"
        );
        bounded_query(sqlx::query("CALL DOLT_RESET('--hard')").fetch_all(main)).await?;
        validate_inspection(main).await
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// Task 4.1: the in-progress attempt's records must name its own branch, its
/// exact base and (for the introducing step) exactly the open's classified
/// publications; a completed attempt that disagrees is never published.
#[tokio::test]
async fn validate_attempt_rejects_record_that_disagrees_with_name_base_receipt_or_digest()
-> Result<()> {
    #[derive(Clone, Copy, Debug)]
    enum Wrong {
        Branch,
        Base,
        Backfill,
    }
    for wrong in [Wrong::Branch, Wrong::Base, Wrong::Backfill] {
        let root = crate::test_support::tempdir()?;
        let options = released_v7(root.path(), '7').await?;
        let server = super::super::tests::released_server(&options).await?;
        let main = server.pool("main").await?;
        let checked = async {
            let base = revision(&main).await?;
            let classified =
                classify_historical_attempts_in(REGISTRY, &main, 7, RESERVED_PREFIX).await?;
            let operation = Uuid::new_v4();
            let name = attempt_name(8, operation);
            branch_call(&main, &[&name, &base]).await?;
            let other_name = attempt_name(8, Uuid::new_v4());
            let (_, v6_head) = sole_branch(&main, 6).await?;
            let partial = &classified.published[1..];
            let record = match wrong {
                Wrong::Branch => AttemptRecord {
                    branch: &other_name,
                    base: &base,
                    published: &classified.published,
                },
                Wrong::Base => AttemptRecord {
                    branch: &name,
                    base: &v6_head,
                    published: &classified.published,
                },
                Wrong::Backfill => AttemptRecord {
                    branch: &name,
                    base: &base,
                    published: partial,
                },
            };
            let attempt = server.pool(&name).await?;
            let built = build_attempt(
                REGISTRY,
                &attempt,
                &V8,
                operation,
                &record,
                &MigrationRunnerHooks::none(),
            )
            .await;
            after_cleanup(built, close_branch_pool(&attempt).await)?;
            // The server caches a branch pool while a handle to it lives.
            drop(attempt);
            let names = vec![name.clone()];
            let attempt_before = snapshot_attempts(&server, &names).await?;
            let before = durable_snapshot(&main).await?;
            assert_failed_runner_unchanged(
                REGISTRY,
                &server,
                &main,
                &before,
                "publication records differ",
            )
            .await?;
            assert_attempts_unchanged(&server, attempt_before).await?;
            Ok(())
        }
        .await;
        main.close().await;
        drop(main);
        after_cleanup(checked, server.close().await).with_context(|| format!("{wrong:?}"))?;
    }
    Ok(())
}

/// Task 3.3: a completed v8 attempt from an interrupted open is reused only
/// when its backfill equals this open's classification; after a published
/// branch was deleted it disagrees, and nothing is published or changed.
#[tokio::test]
async fn reused_completed_v8_attempt_with_disagreeing_backfill_fails_closed() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = released_v7(root.path(), '8').await?;
    let server = super::super::tests::released_server(&options).await?;
    let main = server.pool("main").await?;
    let checked = async {
        let base = revision(&main).await?;
        let classified =
            classify_historical_attempts_in(REGISTRY, &main, 7, RESERVED_PREFIX).await?;
        let operation = Uuid::new_v4();
        let name = attempt_name(8, operation);
        branch_call(&main, &[&name, &base]).await?;
        let attempt = server.pool(&name).await?;
        let built = build_attempt(
            REGISTRY,
            &attempt,
            &V8,
            operation,
            &AttemptRecord {
                branch: &name,
                base: &base,
                published: &classified.published,
            },
            &MigrationRunnerHooks::none(),
        )
        .await;
        after_cleanup(built, close_branch_pool(&attempt).await)?;
        // The server caches a branch pool while a handle to it lives.
        drop(attempt);
        // The completed attempt as an interrupted open left it is sound.
        let attempt = server.pool(&name).await?;
        let sound = validate_attempt(
            REGISTRY,
            &attempt,
            &V8,
            operation,
            &AttemptRecord {
                branch: &name,
                base: &base,
                published: &classified.published,
            },
        )
        .await;
        after_cleanup(sound, close_branch_pool(&attempt).await)?;
        drop(attempt);
        // A published branch goes away before the next open.
        let (v3, _) = sole_branch(&main, 3).await?;
        branch_call(&main, &["-D", &v3]).await?;
        let names = vec![name.clone()];
        let attempt_before = snapshot_attempts(&server, &names).await?;
        let before: DurableSnapshot = durable_snapshot(&main).await?;
        assert_failed_runner_unchanged(
            REGISTRY,
            &server,
            &main,
            &before,
            "publication records differ",
        )
        .await?;
        assert_attempts_unchanged(&server, attempt_before).await?;
        ensure!(version(&main).await? == 7, "the refused attempt published");
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    after_cleanup(checked, server.close().await)
}

/// The test-only v11 step records its own publication like every step after
/// the introducing one: its commit holds main's v2..v10 records and its own.
#[tokio::test]
async fn later_step_records_itself_over_the_base_records() -> Result<()> {
    let store = MemoryStore::temporary_cold().await?;
    let checked = async {
        let main = store.pool.as_ref();
        let base_records = records_in(main).await?;
        upgrade_with(
            TEST_REGISTRY,
            &store.shared.server,
            main,
            &MigrationRunnerHooks::none(),
        )
        .await?;
        validate_active_with(TEST_REGISTRY, main).await?;
        let (v11, v11_head) = sole_branch(main, 11).await?;
        let records = records_in(main).await?;
        ensure!(
            records[..records.len() - 1] == base_records[..],
            "the v11 step changed earlier records"
        );
        let own = records.last().context("no v11 record")?;
        ensure!(
            own.version == 11
                && own.branch == v11
                && own.base == sole_parent(main, &v11_head).await?,
            "the v11 record differs: {own:?}"
        );
        let classified =
            classify_historical_attempts_in(TEST_REGISTRY, main, 11, RESERVED_PREFIX).await?;
        ensure!(
            classified.full == 0 && classified.by_record == 10,
            "classified {classified:?}"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// A store copied from the store template carries one record per retained
/// main branch, accepted by record with no full classification, and the
/// template shape's record check refuses a copy with one record removed.
#[tokio::test]
async fn template_born_store_carries_records_and_passes_the_shape_check() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let checked = async {
        let main = store.pool.as_ref();
        let classified = classify(main).await.map_err(anyhow::Error::msg)?;
        ensure!(
            classified.full == 0 && classified.by_record == usize::try_from(CURRENT_VERSION - 1)?,
            "a template copy classified {classified:?}"
        );
        expected_records(main, &classified.published).await?;
        let attempts = template_shape::Attempts {
            main: classified
                .published
                .iter()
                .map(|published| (published.version, published.operation))
                .collect(),
            usage: BTreeMap::new(),
        };
        template_shape::publication_records(main, &attempts).await?;
        on_branch(
            main,
            "main",
            &[
                "DELETE FROM kuru_migration_publications WHERE version = 4".to_owned(),
                "CALL DOLT_COMMIT('-am', 'P6 drop a record', '--author', 'Kuru <memory@kuru.local>')"
                    .to_owned(),
            ],
        )
        .await?;
        let refused = template_shape::publication_records(main, &attempts)
            .await
            .expect_err("a template without every record was accepted");
        ensure!(
            crate::server::TemplateVerdict::find(&refused).is_some(),
            "a missing record is not a template verdict: {refused:#}"
        );
        Ok(())
    }
    .await;
    after_cleanup(checked, store.close().await)
}

/// In-process cost of `validate_active` on a v8 store with seven recorded
/// branches, verified by record, against the same store with records ignored
/// (full classification), interleaved. Report only; run explicitly:
/// `cargo test -p kuru-memory --lib --all-features measure_publication_records -- --ignored --nocapture`
#[tokio::test]
#[ignore = "measurement, run explicitly with --ignored --nocapture"]
async fn measure_publication_records() -> Result<()> {
    let iterations: usize = std::env::var("KURU_TEST_P6_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7);
    let store = MemoryStore::temporary_cold().await?;
    let measured = async {
        let main = store.pool.as_ref();
        let full = || async {
            validate_current_with(REGISTRY, main).await?;
            clean(main).await?;
            classify_historical_attempts_with(
                REGISTRY,
                main,
                REGISTRY.current,
                RESERVED_PREFIX,
                Records::Ignore,
            )
            .await
            .map(|_| ())
        };
        validate_active(main).await?;
        full().await?;
        let (mut by_record, mut ignored) = (Vec::new(), Vec::new());
        for _ in 0..iterations {
            let started = Instant::now();
            full().await?;
            ignored.push(started.elapsed().as_secs_f64() * 1000.0);
            let started = Instant::now();
            validate_active(main).await?;
            by_record.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let median = |values: &[f64]| {
            let mut values = values.to_vec();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        eprintln!(
            "P6 validate_active ms, {} retained branches, N={iterations}: records ignored {ignored:.1?} (median {:.1}); by record {by_record:.1?} (median {:.1})",
            reserved_names(main).await?.len(),
            median(&ignored),
            median(&by_record),
        );
        Ok(())
    }
    .await;
    after_cleanup(measured, store.close().await)
}
