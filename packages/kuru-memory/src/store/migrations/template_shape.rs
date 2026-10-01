//! The shape of a store template, checked on a live engine.
//!
//! A store template is the `data/` of a store built by the migration chain
//! under a fixed placeholder identity. It holds schema, migration receipts
//! and that identity, and nothing else. One function, [`check`], asserts it:
//! the template build runs it before publication with the placeholder row,
//! and the first engine of a stage copied from a template runs it after
//! adoption with the stage's own row. A bug in the check therefore refuses a
//! build rather than condemning a published template at every copy.
//!
//! Expectations derive from the compiled migration registries, never from
//! literals: `main` holds [`BASE_COMMITS`] and one commit per main step; the
//! usage branch holds the base, the main steps up to its anchor and its own
//! steps beyond it; one retained attempt branch exists per executed step and
//! namespace. An adopted copy adds one commit on each of the two refs.
//!
//! Every commit's committer, email, author, author email and message are
//! asserted on both refs as well: Dolt's own first commit under the engine's
//! fixed system account, then Kuru's commits under [`AUTHOR`] with the
//! initialization message, one upgrade message per step naming the retained
//! attempt's operation, and the adoption message. No commit can carry a host
//! name, an operating-system user name or a path, which the capture's byte
//! scan cannot rule out in compressed chunks (engine contract finding S7).
//!
//! A completed query that returns another value is a [`TemplateVerdict`]. A
//! query error or deadline is an ordinary error and says nothing about the
//! template. Schema validation and historical classification keep their own
//! ordinary errors: a refusal there never condemns a template.
//!
//! Retained main attempts are classified here from `main`, as every open
//! classifies them. Retained usage attempts are only counted by the branch
//! set: their classification compares with the usage branch's own head, so it
//! runs from the usage pool (`validate_usage`), which the build runs and every
//! writable open runs when it establishes the usage ledger.
use super::*;
use crate::server::{ADOPTION_MESSAGE, TEMPLATE_INSTANCE, TEMPLATE_SCOPE, TemplateVerdict};
use crate::store::INITIALIZE_MESSAGE;
use std::collections::BTreeMap;

/// Commits a store holds before its first schema step: Dolt's
/// `Initialize data repository` and Kuru's `Initialize Kuru memory schema 1`.
/// Measured on the pinned Dolt 2.3.5 (a cold store's `dolt_log` on `main`
/// and on the usage branch); a template-to-cold parity check pins it once
/// templates are built.
const BASE_COMMITS: u64 = 2;
/// Dolt's own first commit, written by `CREATE DATABASE` under the engine's
/// fixed system account whatever the host, the user or the private home
/// hold. Measured on the pinned Dolt 2.3.5 (`dolt_log` of a built template on
/// `main` and on the usage branch).
const DOLT_INITIAL_COMMITTER: &str = "Dolt System Account";
const DOLT_INITIAL_EMAIL: &str = "doltuser@dolthub.com";
const DOLT_INITIAL_MESSAGE: &str = "Initialize data repository";
/// The schema version at which a new usage branch is anchored: main's clean
/// schema-4 head, before main takes schema 5 (`ensure_usage_branch_at_v4`).
const USAGE_ANCHOR: i32 = 4;
/// Tables whose rows are schema, receipt, publication-record and identity
/// authority. Every other table on `main` and on the usage branch is project
/// data and must be empty.
const AUTHORITY_TABLES: [&str; 4] = [
    "kuru_instance",
    "kuru_migration_publications",
    "kuru_migrations",
    "kuru_schema",
];
/// The most branches a template may hold: the two refs and one retained
/// attempt per step of each registry. One more is read, so an overflow is a
/// verdict rather than a silently truncated branch set.
const BRANCH_LIMIT: usize = 2 * DEFINITION_LIMIT + 2;
/// The most tables a ref of a template may hold. One more is read, so a table
/// beyond the bound is a verdict rather than one the check never counted.
const TABLE_LIMIT: usize = 256;

/// The identity row the shape requires on `main` and on the usage branch.
#[derive(Clone, Copy, Debug)]
pub(in crate::store) enum Row<'a> {
    /// A template before adoption: the compiled placeholder.
    Placeholder,
    /// A copy after adoption: its own instance and project scope, and one
    /// adoption commit on each ref.
    Adopted {
        instance: &'a str,
        project_scope: &'a str,
    },
}

impl Row<'_> {
    fn values(&self) -> (&str, &str) {
        match self {
            Self::Placeholder => (TEMPLATE_INSTANCE, TEMPLATE_SCOPE),
            Self::Adopted {
                instance,
                project_scope,
            } => (instance, project_scope),
        }
    }

    fn adopted(&self) -> bool {
        matches!(self, Self::Adopted { .. })
    }
}

/// The commit counts and retained attempt targets a template of these
/// registries holds.
#[derive(Debug, Eq, PartialEq)]
struct Expected {
    main_commits: u64,
    usage_commits: u64,
    /// Targets of the retained main attempt branches, ascending.
    main_attempts: Vec<i32>,
    /// Targets of the retained usage attempt branches, ascending.
    usage_attempts: Vec<i32>,
}

fn expected(main: Registry, usage: Registry, anchor: i32, adopted: bool) -> Result<Expected> {
    main.validate()?;
    usage.validate()?;
    let adoption = u64::from(adopted);
    let steps = |definitions: &mut dyn Iterator<Item = &Definition>| -> Result<u64> {
        Ok(u64::try_from(definitions.count())?)
    };
    let main_steps = steps(&mut main.definitions.iter())?;
    let shared_steps = steps(&mut main.definitions.iter().filter(|step| step.to <= anchor))?;
    let usage_steps = steps(&mut usage.definitions.iter().filter(|step| step.to > anchor))?;
    Ok(Expected {
        main_commits: BASE_COMMITS + main_steps + adoption,
        usage_commits: BASE_COMMITS + shared_steps + usage_steps + adoption,
        main_attempts: main.definitions.iter().map(|step| step.to).collect(),
        usage_attempts: usage
            .definitions
            .iter()
            .filter(|step| step.to > anchor)
            .map(|step| step.to)
            .collect(),
    })
}

fn verdict(reason: String) -> anyhow::Error {
    TemplateVerdict::new(format!("memory store template shape: {reason}")).into()
}

/// The `LIMIT` bound for a list that may hold at most `limit` rows: one more,
/// so [`within`] can tell a complete list from a truncated one.
fn read_limit(limit: usize) -> Result<i64> {
    Ok(i64::try_from(limit)?.saturating_add(1))
}

/// A completed list read with [`read_limit`] that returned more than `limit`
/// rows holds more than a template may: a verdict, never a truncation.
fn within(found: usize, limit: usize, what: &str, reference: &str) -> Result<()> {
    if found > limit {
        return Err(verdict(format!(
            "{reference} holds more than {limit} {what}"
        )));
    }
    Ok(())
}

/// Commits on `main` and on the usage branch that the compiled registries
/// expect, before (`adopted == false`) or after adoption.
#[cfg(test)]
pub(in crate::store) fn compiled_commits(adopted: bool) -> Result<(u64, u64)> {
    let expected = expected(REGISTRY, USAGE_REGISTRY, USAGE_ANCHOR, adopted)?;
    Ok((expected.main_commits, expected.usage_commits))
}

/// An engine-supplied name, bounded for a verdict message.
fn shown(name: &str) -> String {
    format!("{:?}", name.chars().take(64).collect::<String>())
}

/// A ref the template shape reads.
#[derive(Clone, Copy)]
enum Ref {
    Main,
    Usage,
}

impl Ref {
    fn name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Usage => "usage branch",
        }
    }

    fn registry(self) -> Registry {
        match self {
            Self::Main => REGISTRY,
            Self::Usage => USAGE_REGISTRY,
        }
    }

    /// The `USE` statement selecting this ref: only these two constant texts.
    fn select(self) -> &'static str {
        match self {
            Self::Main => "USE `kuru`",
            Self::Usage => "USE `kuru/kuru_usage_v1`",
        }
    }
}

/// Assert the template shape of the store `main` serves, with `row` as its
/// identity on `main` and on the usage branch. See the module documentation
/// for what is a verdict and what is not. It classifies the retained main
/// attempts itself, so a caller validating the store first uses a validation
/// that does not classify them again.
pub(in crate::store) async fn check(main: &MySqlPool, row: Row<'_>) -> Result<()> {
    let expected = expected(REGISTRY, USAGE_REGISTRY, USAGE_ANCHOR, row.adopted())?;
    let attempts = branches(main, &expected).await?;
    classify_historical_attempts(REGISTRY, main, REGISTRY.current).await?;
    publication_records(main, &attempts).await?;
    let mut connection = acquire(main).await?.detach();
    let checked = async {
        for (reference, commits) in [
            (Ref::Main, expected.main_commits),
            (Ref::Usage, expected.usage_commits),
        ] {
            let history = expected_history(
                REGISTRY,
                USAGE_REGISTRY,
                USAGE_ANCHOR,
                reference,
                &attempts,
                row.adopted(),
            )?;
            ensure!(
                u64::try_from(history.len())? == commits,
                "the expected {} history has {} commits, not {commits}",
                reference.name(),
                history.len()
            );
            reference_shape(&mut connection, reference, &history, row).await?;
        }
        Ok(())
    }
    .await;
    after_cleanup(checked, bounded_query(connection.close()).await)
}

/// The operations of the retained attempt branches, by target version.
#[derive(Debug, Default)]
pub(super) struct Attempts {
    pub(super) main: BTreeMap<i32, Uuid>,
    pub(super) usage: BTreeMap<i32, Uuid>,
}

/// The exact branch set: `main`, the usage branch and one clean retained
/// attempt per executed step and namespace. Returns each attempt's
/// operation, which its published commit's message names.
async fn branches(main: &MySqlPool, expected: &Expected) -> Result<Attempts> {
    let rows: Vec<(String, bool)> = bounded_query(
        sqlx::query_as("SELECT name, dirty FROM dolt_branches ORDER BY name LIMIT ?")
            .bind(read_limit(BRANCH_LIMIT)?)
            .fetch_all(main),
    )
    .await?;
    within(rows.len(), BRANCH_LIMIT, "branches", "the store")?;
    let mut refs = BTreeSet::new();
    let mut main_attempts = Vec::new();
    let mut usage_attempts = Vec::new();
    for (name, dirty) in rows {
        if dirty {
            return Err(verdict(format!(
                "branch {} has uncommitted changes",
                shown(&name)
            )));
        }
        if name == "main" || name == super::super::usage_ledger::BRANCH {
            refs.insert(name);
        } else if let Ok(attempt) = parse_attempt_in(USAGE_RESERVED_PREFIX, &name) {
            usage_attempts.push(attempt);
        } else if let Ok(attempt) = parse_attempt_in(RESERVED_PREFIX, &name) {
            main_attempts.push(attempt);
        } else {
            return Err(verdict(format!("unexpected branch {}", shown(&name))));
        }
    }
    main_attempts.sort_unstable();
    usage_attempts.sort_unstable();
    if refs.len() != 2 {
        return Err(verdict(format!(
            "main or the usage branch is missing (found {refs:?})"
        )));
    }
    let targets = |attempts: &[(i32, Uuid)]| -> Vec<i32> {
        attempts.iter().map(|(target, _)| *target).collect()
    };
    let (main_targets, usage_targets) = (targets(&main_attempts), targets(&usage_attempts));
    if main_targets != expected.main_attempts || usage_targets != expected.usage_attempts {
        return Err(verdict(format!(
            "retained attempts {main_targets:?} and usage attempts {usage_targets:?} differ \
             from the compiled {:?} and {:?}",
            expected.main_attempts, expected.usage_attempts
        )));
    }
    // The targets are distinct: they equal the compiled, strictly
    // increasing ones.
    Ok(Attempts {
        main: main_attempts.into_iter().collect(),
        usage: usage_attempts.into_iter().collect(),
    })
}

/// Exactly one publication record per retained main attempt, naming that
/// attempt: a template-era branch is accepted on later opens by its record,
/// so a template without one for every retained branch is refused. The
/// records themselves were verified against main's history by the
/// classification `check` ran first.
pub(super) async fn publication_records(main: &MySqlPool, attempts: &Attempts) -> Result<()> {
    let expected: Vec<(i32, String)> = attempts
        .main
        .iter()
        .map(|(target, operation)| {
            (
                *target,
                attempt_name_in(RESERVED_PREFIX, *target, *operation),
            )
        })
        .collect();
    let found: Vec<(i32, String)> = records_in(main)
        .await?
        .into_iter()
        .map(|record| (record.version, record.branch))
        .collect();
    if found != expected {
        return Err(verdict(format!(
            "publication records name {:?}, not one per retained attempt {:?}",
            found
                .iter()
                .map(|(version, branch)| format!("{version}:{}", shown(branch)))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>()
        )));
    }
    Ok(())
}

/// One `dolt_log` row as read: committer, email, author, author email and
/// message.
type HistoryRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// One commit of a ref's history, as the shape asserts it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Commit {
    committer: String,
    email: String,
    author: String,
    author_email: String,
    message: String,
}

impl Commit {
    fn new(name: &str, email: &str, message: String) -> Self {
        Self {
            committer: name.to_owned(),
            email: email.to_owned(),
            author: name.to_owned(),
            author_email: email.to_owned(),
            message,
        }
    }
}

/// The name and email of [`AUTHOR`] (`Name <email>`).
fn kuru_author() -> Result<(&'static str, &'static str)> {
    AUTHOR
        .strip_suffix('>')
        .and_then(|author| author.split_once(" <"))
        .context("the compiled commit author is not `Name <email>`")
}

/// The history, oldest first, that a template of these registries holds on
/// `target`: Dolt's first commit, Kuru's initialization, one upgrade per
/// executed step naming its retained attempt's operation (the usage branch
/// shares main's steps up to `anchor`), and the adoption commit once adopted.
fn expected_history(
    main: Registry,
    usage: Registry,
    anchor: i32,
    target: Ref,
    attempts: &Attempts,
    adopted: bool,
) -> Result<Vec<Commit>> {
    let (name, email) = kuru_author()?;
    let [before, middle, after] = ATTEMPT_MESSAGE;
    let upgrade = |to: i32, operations: &BTreeMap<i32, Uuid>| -> Result<Commit> {
        let operation = operations
            .get(&to)
            .with_context(|| format!("no retained attempt for schema {to}"))?;
        Ok(Commit::new(
            name,
            email,
            format!("{before}{to}{middle}{operation}{after}"),
        ))
    };
    let mut history = vec![
        Commit::new(
            DOLT_INITIAL_COMMITTER,
            DOLT_INITIAL_EMAIL,
            DOLT_INITIAL_MESSAGE.to_owned(),
        ),
        Commit::new(name, email, INITIALIZE_MESSAGE.to_owned()),
    ];
    match target {
        Ref::Main => {
            for step in main.definitions {
                history.push(upgrade(step.to, &attempts.main)?);
            }
        }
        Ref::Usage => {
            for step in main.definitions.iter().filter(|step| step.to <= anchor) {
                history.push(upgrade(step.to, &attempts.main)?);
            }
            for step in usage.definitions.iter().filter(|step| step.to > anchor) {
                history.push(upgrade(step.to, &attempts.usage)?);
            }
        }
    }
    if adopted {
        history.push(Commit::new(name, email, ADOPTION_MESSAGE.to_owned()));
    }
    Ok(history)
}

/// A completed history read that differs from the expected one is a verdict.
fn compare_history(reference: &str, found: &[Commit], expected: &[Commit]) -> Result<()> {
    if found.len() != expected.len() {
        return Err(verdict(format!(
            "{reference} history lists {} commits, not {}",
            found.len(),
            expected.len()
        )));
    }
    for (index, (found, expected)) in found.iter().zip(expected).enumerate() {
        for (field, found, expected) in [
            ("committer", &found.committer, &expected.committer),
            ("email", &found.email, &expected.email),
            ("author", &found.author, &expected.author),
            ("author email", &found.author_email, &expected.author_email),
            ("message", &found.message, &expected.message),
        ] {
            if found != expected {
                return Err(verdict(format!(
                    "{reference} commit {} has {field} {}, not {}",
                    index + 1,
                    shown(found),
                    shown(expected)
                )));
            }
        }
    }
    Ok(())
}

/// One ref's schema version, working set, history, identity row, project
/// data and non-table objects, read on a session switched to it.
async fn reference_shape(
    connection: &mut MySqlConnection,
    target: Ref,
    expected_history: &[Commit],
    row: Row<'_>,
) -> Result<()> {
    let commits = u64::try_from(expected_history.len())?;
    let reference = target.name();
    let registry = target.registry();
    bounded_query(connection.execute(target.select())).await?;
    let found = version_on(connection).await?;
    if found != registry.current {
        return Err(verdict(format!(
            "{reference} is at schema {found}, not {}",
            registry.current
        )));
    }
    validate_version_on(registry, connection, found).await?;
    let changes: i64 = bounded_query(
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(&mut *connection),
    )
    .await?;
    if changes != 0 {
        return Err(verdict(format!(
            "{reference} has {changes} uncommitted changes"
        )));
    }
    let history: i64 = bounded_query(
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log").fetch_one(&mut *connection),
    )
    .await?;
    if u64::try_from(history).ok() != Some(commits) {
        return Err(verdict(format!(
            "{reference} holds {history} commits, not {commits}"
        )));
    }
    // Oldest first; `date` and `author_date` are not read.
    let found: Vec<HistoryRow> =
        bounded_query(
            sqlx::query_as(
                "SELECT CAST(committer AS CHAR), CAST(email AS CHAR), CAST(author AS CHAR), CAST(author_email AS CHAR), CAST(message AS CHAR) FROM dolt_log ORDER BY commit_order LIMIT ?",
            )
            .bind(i64::try_from(commits)?)
            .fetch_all(&mut *connection),
        )
        .await?;
    let found: Vec<Commit> = found
        .into_iter()
        .map(|(committer, email, author, author_email, message)| Commit {
            committer: committer.unwrap_or_default(),
            email: email.unwrap_or_default(),
            author: author.unwrap_or_default(),
            author_email: author_email.unwrap_or_default(),
            message: message.unwrap_or_default(),
        })
        .collect();
    compare_history(reference, &found, expected_history)?;
    let rows: Vec<(String, String)> = bounded_query(
        sqlx::query_as(
            "SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1 LIMIT 2",
        )
        .fetch_all(&mut *connection),
    )
    .await?;
    let (instance, scope) = row.values();
    if !matches!(rows.as_slice(), [(found_instance, found_scope)] if found_instance == instance && found_scope == scope)
    {
        return Err(verdict(format!(
            "{reference} does not hold exactly the expected identity row ({} rows)",
            rows.len()
        )));
    }
    let tables: Vec<(String, String)> = bounded_query(
        sqlx::query_as(
            "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR) FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name LIMIT ?",
        )
        .bind(read_limit(TABLE_LIMIT)?)
        .fetch_all(&mut *connection),
    )
    .await?;
    within(tables.len(), TABLE_LIMIT, "tables", reference)?;
    for (table, kind) in tables {
        if kind != "BASE TABLE" {
            return Err(verdict(format!(
                "{reference} holds the {} {}",
                shown(&kind),
                shown(&table)
            )));
        }
        if AUTHORITY_TABLES.contains(&table.as_str()) {
            continue;
        }
        if table.is_empty()
            || table.len() > 64
            || !table
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(verdict(format!(
                "{reference} holds an unexpected table {}",
                shown(&table)
            )));
        }
        // The name was checked against `[a-z0-9_]{1,64}` above.
        let count: i64 = bounded_query(
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM `{table}`"
            )))
            .fetch_one(&mut *connection),
        )
        .await?;
        if count != 0 {
            return Err(verdict(format!(
                "{reference} table {table:?} holds {count} rows of project data"
            )));
        }
    }
    for (objects, count) in non_table_objects(connection).await? {
        if count != 0 {
            return Err(verdict(format!("{reference} holds {count} {objects}")));
        }
    }
    Ok(())
}

/// The non-table objects a template may not hold, counted on the ref the
/// session is switched to: views, triggers and routines from
/// `information_schema`, and the Dolt system tables that store schema
/// objects, procedures and ignore rules. On the pinned engine every one of
/// these reads answers on a store that never held such an object, so an
/// absent object counts as zero; a read that fails is an ordinary error.
pub(in crate::store) async fn non_table_objects(
    connection: &mut MySqlConnection,
) -> Result<Vec<(&'static str, i64)>> {
    let mut counts = Vec::new();
    for (objects, query) in [
        (
            "views",
            "SELECT COUNT(*) FROM information_schema.views WHERE table_schema = DATABASE()",
        ),
        (
            "triggers",
            "SELECT COUNT(*) FROM information_schema.triggers WHERE trigger_schema = DATABASE()",
        ),
        (
            "routines",
            "SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema = DATABASE()",
        ),
        ("stored schema objects", "SELECT COUNT(*) FROM dolt_schemas"),
        ("stored procedures", "SELECT COUNT(*) FROM dolt_procedures"),
        ("ignore rules", "SELECT COUNT(*) FROM dolt_ignore"),
    ] {
        let count: i64 =
            bounded_query(sqlx::query_scalar(query).fetch_one(&mut *connection)).await?;
        counts.push((objects, count));
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::INITIALIZE_COMMIT;

    const fn step(from: i32, id: &'static str) -> Definition {
        Definition {
            from,
            to: from + 1,
            id,
            sql: &["SELECT 1"],
            transform: "none",
            postcondition: "none",
            failed_status: &[],
        }
    }

    /// Counts and attempt targets follow the registries, including a
    /// usage-only step beyond the anchor, and adoption adds one commit per
    /// ref; the compiled registries give the measured cold-store shape.
    #[test]
    fn template_shape_counts_derive_from_registries() -> Result<()> {
        const MAIN: &[Definition] = &[
            step(1, "a"),
            step(2, "b"),
            step(3, "c"),
            step(4, "d"),
            step(5, "e"),
        ];
        const USAGE: &[Definition] = &[step(1, "a"), step(2, "b"), step(3, "c"), step(4, "u")];
        let main = Registry {
            current: 6,
            definitions: MAIN,
        };
        let usage = Registry {
            current: 5,
            definitions: USAGE,
        };
        assert_eq!(
            expected(main, usage, 3, false)?,
            Expected {
                // Base, then five main steps.
                main_commits: BASE_COMMITS + 5,
                // Base, the two main steps up to the anchor, then the usage
                // steps beyond it.
                usage_commits: BASE_COMMITS + 2 + 2,
                main_attempts: vec![2, 3, 4, 5, 6],
                usage_attempts: vec![4, 5],
            }
        );
        let adopted = expected(main, usage, 3, true)?;
        assert_eq!(adopted.main_commits, BASE_COMMITS + 6);
        assert_eq!(adopted.usage_commits, BASE_COMMITS + 5);

        // The usage ref's selection names the usage ledger's branch.
        assert_eq!(
            Ref::Usage.select(),
            format!(
                "USE `{DATABASE}/{}`",
                super::super::super::usage_ledger::BRANCH
            )
        );
        assert_eq!(Ref::Main.select(), format!("USE `{DATABASE}`"));
        // The anchor is the schema a new usage branch is created at.
        assert_eq!(V5.from, USAGE_ANCHOR);
        // The compiled registries are consecutive from schema 1 (their
        // `validate`), so each ref holds the base and one commit per version
        // step to its current schema, whatever the current schemas are. The
        // cold store the counts were measured on (eight commits on `main`,
        // five on the usage branch) is the build of every template test.
        let compiled = expected(REGISTRY, USAGE_REGISTRY, USAGE_ANCHOR, false)?;
        let steps = |current: i32| u64::try_from(current - 1);
        assert_eq!(
            compiled.main_commits,
            BASE_COMMITS + steps(CURRENT_VERSION)?
        );
        assert_eq!(
            compiled.usage_commits,
            BASE_COMMITS + steps(USAGE_CURRENT_VERSION)?
        );
        assert_eq!(
            compiled.main_attempts,
            (2..=CURRENT_VERSION).collect::<Vec<_>>()
        );
        assert_eq!(
            compiled_commits(true)?,
            (compiled.main_commits + 1, compiled.usage_commits + 1)
        );
        // `check` counts retained usage attempts but classifies only main
        // attempts; usage attempts are classified by `validate_usage` from
        // the usage pool. None exists while the usage registry ends at its
        // anchor. A usage step beyond it must first make the build and the
        // stage engine run that classification before `ready.json`.
        assert!(
            compiled.usage_attempts.is_empty(),
            "a usage schema step now leaves retained usage attempts: classify them \
             (validate_usage) in the template build and the stage engine first"
        );

        // A list read one row past its bound is a verdict, not a truncation.
        assert!(within(TABLE_LIMIT, TABLE_LIMIT, "tables", "main").is_ok());
        let overflow = within(TABLE_LIMIT + 1, TABLE_LIMIT, "tables", "main")
            .expect_err("an overflowing list was accepted");
        assert!(TemplateVerdict::find(&overflow).is_some());
        assert_eq!(read_limit(BRANCH_LIMIT)?, 2 * DEFINITION_LIMIT as i64 + 3);

        // An invalid registry is refused rather than counted.
        let broken = Registry {
            current: 9,
            definitions: MAIN,
        };
        assert!(expected(broken, usage, 3, false).is_err());
        Ok(())
    }

    /// Every commit's identity and message is derived: Dolt's system
    /// account first, then Kuru's author with the initialization, one
    /// upgrade per step naming its retained attempt's operation, and the
    /// adoption; any other committer, email, author or message is a verdict,
    /// and a missing attempt is an ordinary error.
    #[test]
    fn template_history_derives_every_commit_identity() -> Result<()> {
        const MAIN: &[Definition] = &[step(1, "a"), step(2, "b"), step(3, "c"), step(4, "d")];
        const USAGE: &[Definition] = &[step(1, "a"), step(2, "b"), step(3, "u")];
        let main = Registry {
            current: 5,
            definitions: MAIN,
        };
        let usage = Registry {
            current: 4,
            definitions: USAGE,
        };
        let operation = Uuid::from_u128;
        let attempts = Attempts {
            main: (2..=5)
                .map(|to: i32| (to, operation(u128::from(to.unsigned_abs()))))
                .collect(),
            usage: [(4, operation(40))].into_iter().collect(),
        };
        let (name, email) = kuru_author()?;
        assert_eq!((name, email), ("Kuru", "memory@kuru.local"));
        assert!(INITIALIZE_COMMIT.contains(&format!("'{INITIALIZE_MESSAGE}'")));
        let messages = |history: &[Commit]| -> Vec<String> {
            history
                .iter()
                .map(|commit| commit.message.clone())
                .collect()
        };
        let upgrade =
            |to: i32, operation: Uuid| format!("Upgrade Kuru memory schema {to} [{operation}]");
        let on_main = expected_history(main, usage, 3, Ref::Main, &attempts, false)?;
        assert_eq!(
            messages(&on_main),
            [
                DOLT_INITIAL_MESSAGE.to_owned(),
                INITIALIZE_MESSAGE.to_owned(),
                upgrade(2, operation(2)),
                upgrade(3, operation(3)),
                upgrade(4, operation(4)),
                upgrade(5, operation(5)),
            ]
        );
        assert_eq!(
            on_main[0],
            Commit::new(
                "Dolt System Account",
                "doltuser@dolthub.com",
                "Initialize data repository".into()
            )
        );
        assert!(on_main[1..].iter().all(|commit| commit.committer == name
            && commit.author == name
            && commit.email == email
            && commit.author_email == email));
        let on_usage = expected_history(main, usage, 3, Ref::Usage, &attempts, false)?;
        assert_eq!(
            messages(&on_usage)[2..],
            [
                upgrade(2, operation(2)),
                upgrade(3, operation(3)),
                upgrade(4, operation(40))
            ]
        );
        let counts = expected(main, usage, 3, false)?;
        assert_eq!(on_main.len() as u64, counts.main_commits);
        assert_eq!(on_usage.len() as u64, counts.usage_commits);
        for reference in [Ref::Main, Ref::Usage] {
            let adopted = expected_history(main, usage, 3, reference, &attempts, true)?;
            assert_eq!(
                adopted.last(),
                Some(&Commit::new(name, email, ADOPTION_MESSAGE.into()))
            );
        }
        compare_history("main", &on_main, &on_main)?;
        // Each field, and the length, is compared.
        type Edit = fn(&mut Commit);
        let edits: [(&str, Edit); 5] = [
            ("committer", |commit| commit.committer = "runner".into()),
            ("email", |commit| commit.email = "runner@build-host".into()),
            ("author", |commit| commit.author = "runner".into()),
            ("author email", |commit| {
                commit.author_email = "runner@build-host".into()
            }),
            ("message", |commit| {
                commit.message = "Upgrade Kuru memory schema 3 [another operation]".into();
            }),
        ];
        for (field, edit) in edits {
            for index in [0, 3] {
                let mut found = on_main.clone();
                edit(&mut found[index]);
                let refused = compare_history("main", &found, &on_main)
                    .expect_err("a differing commit was accepted");
                assert!(
                    TemplateVerdict::find(&refused).is_some(),
                    "{field}: {refused:#}"
                );
                assert!(format!("{refused:#}").contains(field), "{refused:#}");
            }
        }
        let refused = compare_history("main", &on_main[1..], &on_main)
            .expect_err("a shorter history was accepted");
        assert!(TemplateVerdict::find(&refused).is_some());
        // A retained attempt the branch set did not report is not a verdict.
        let missing = Attempts {
            main: attempts.main.clone(),
            usage: BTreeMap::new(),
        };
        let error = expected_history(main, usage, 3, Ref::Usage, &missing, false)
            .expect_err("a missing attempt was accepted");
        assert!(TemplateVerdict::find(&error).is_none());
        Ok(())
    }
}
