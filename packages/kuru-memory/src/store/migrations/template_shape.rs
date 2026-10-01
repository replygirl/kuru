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
use crate::server::{TEMPLATE_INSTANCE, TEMPLATE_SCOPE, TemplateVerdict};

/// Commits a store holds before its first schema step: Dolt's
/// `Initialize data repository` and Kuru's `Initialize Kuru memory schema 1`.
/// Measured on the pinned Dolt 2.3.5 (a cold store's `dolt_log` on `main`
/// and on the usage branch); a template-to-cold parity check pins it once
/// templates are built.
const BASE_COMMITS: u64 = 2;
/// The schema version at which a new usage branch is anchored: main's clean
/// schema-4 head, before main takes schema 5 (`ensure_usage_branch_at_v4`).
const USAGE_ANCHOR: i32 = 4;
/// Tables whose rows are schema, receipt and identity authority. Every other
/// table on `main` and on the usage branch is project data and must be empty.
const AUTHORITY_TABLES: [&str; 3] = ["kuru_instance", "kuru_migrations", "kuru_schema"];
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
    branches(main, &expected).await?;
    classify_historical_attempts(REGISTRY, main, REGISTRY.current).await?;
    let mut connection = acquire(main).await?.detach();
    let checked = async {
        for (reference, commits) in [
            (Ref::Main, expected.main_commits),
            (Ref::Usage, expected.usage_commits),
        ] {
            reference_shape(&mut connection, reference, commits, row).await?;
        }
        Ok(())
    }
    .await;
    after_cleanup(checked, bounded_query(connection.close()).await)
}

/// The exact branch set: `main`, the usage branch and one clean retained
/// attempt per executed step and namespace.
async fn branches(main: &MySqlPool, expected: &Expected) -> Result<()> {
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
        } else if let Ok((target, _)) = parse_attempt_in(USAGE_RESERVED_PREFIX, &name) {
            usage_attempts.push(target);
        } else if let Ok((target, _)) = parse_attempt_in(RESERVED_PREFIX, &name) {
            main_attempts.push(target);
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
    if main_attempts != expected.main_attempts || usage_attempts != expected.usage_attempts {
        return Err(verdict(format!(
            "retained attempts {main_attempts:?} and usage attempts {usage_attempts:?} differ \
             from the compiled {:?} and {:?}",
            expected.main_attempts, expected.usage_attempts
        )));
    }
    Ok(())
}

/// One ref's schema version, working set, history length, identity row,
/// project data and non-table objects, read on a session switched to it.
async fn reference_shape(
    connection: &mut MySqlConnection,
    target: Ref,
    commits: u64,
    row: Row<'_>,
) -> Result<()> {
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
}
