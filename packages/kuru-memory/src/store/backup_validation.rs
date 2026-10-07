//! SQL observations of one independently restored dataset. Native fsck owns
//! chunk/history validation; this reader covers retained refs and working sets
//! without committing, resetting or changing copied application records.

use anyhow::{Context, Result, ensure};
use futures::TryStreamExt;
use sqlx::{Column, Connection, Executor, MySqlConnection, Row, SqlSafeStr, Statement};

use crate::{
    backup::{BackupCancellation, BackupRef, BackupRefKind, BackupWorkingSet},
    server::Server,
};

pub(crate) struct Inspection {
    pub(crate) schema_version: i32,
    pub(crate) refs: Vec<BackupRef>,
}

pub(crate) async fn inspect(
    server: &Server,
    cancellation: &BackupCancellation,
) -> Result<Inspection> {
    let main = server.pool("main").await?;
    let schema_version = super::migrations::validate_backup_main(&main).await?;
    let mut refs = Vec::new();
    let mut retained_bytes = 0_u64;
    // These are fixed native tables and columns, never caller SQL. Each row
    // contains only bounded ref metadata, not a private application body.
    for (kind, statement) in [
        (
            BackupRefKind::Branch,
            "SELECT name, hash, dirty FROM dolt_branches",
        ),
        (
            BackupRefKind::Tag,
            "SELECT tag_name, tag_hash, 0 FROM dolt_tags",
        ),
        (
            BackupRefKind::Remote,
            "SELECT name, hash, 0 FROM dolt_remote_branches",
        ),
    ] {
        let mut rows = sqlx::query(sqlx::AssertSqlSafe(statement)).fetch(main.as_ref());
        while let Some(row) = rows.try_next().await? {
            ensure!(!cancellation.is_cancelled(), "backup validation cancelled");
            let name: String = row.try_get(0)?;
            let head: String = row.try_get(1)?;
            let dirty: i64 = row.try_get(2)?;
            ensure!(
                !name.is_empty() && name.len() <= 4096 && !name.chars().any(char::is_control),
                "native ref name exceeds its checked metadata bound"
            );
            ensure!(
                crate::backup::noms_hash(&head),
                "native ref head is malformed"
            );
            retained_bytes = retained_bytes
                .checked_add(u64::try_from(name.len() + head.len() + 256)?)
                .context("native ref metadata size overflow")?;
            ensure!(
                retained_bytes <= 32 * 1024 * 1024,
                "native ref observations exceed the backup metadata bound"
            );
            refs.push(BackupRef {
                kind,
                name,
                head,
                working: (kind == BackupRefKind::Branch).then(|| BackupWorkingSet {
                    working_root: String::new(),
                    staged_root: String::new(),
                    dirty: dirty != 0,
                }),
            });
        }
    }
    refs.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    ensure!(
        refs.windows(2)
            .all(|pair| (pair[0].kind, &pair[0].name) < (pair[1].kind, &pair[1].name)),
        "native ref observations are duplicated"
    );
    if refs.iter().any(|reference| {
        reference.kind == BackupRefKind::Branch && reference.name == super::usage_ledger::BRANCH
    }) {
        let usage = server.pool(super::usage_ledger::BRANCH).await?;
        super::migrations::validate_backup_usage(&usage).await?;
    }
    let mut connection = main.acquire().await?.detach();
    let outcome = inspect_refs(&mut connection, &mut refs, cancellation).await;
    let closed = connection.close().await;
    drop(main);
    outcome?;
    closed.context("close native image read session")?;
    Ok(Inspection {
        schema_version,
        refs,
    })
}

async fn inspect_refs(
    connection: &mut MySqlConnection,
    refs: &mut [BackupRef],
    cancellation: &BackupCancellation,
) -> Result<()> {
    let mut read_roots = std::collections::BTreeSet::new();
    for (ref_index, reference) in refs.iter_mut().enumerate() {
        ensure!(!cancellation.is_cancelled(), "backup validation cancelled");
        select(connection, &reference.head, &reference.head).await?;
        // Native AS OF HEAD resolves the repository head ref even on a
        // commit-qualified database. Use the captured immutable commit here;
        // WORKING/STAGED below deliberately resolve the selected branch.
        let head_tables = tables(connection, &reference.head).await?;
        // Arbitrary native refs can precede application initialization. They
        // remain graph/read validated; refs carrying Kuru authority use its
        // released dispatcher, including the independent usage registry.
        if head_tables.iter().any(|table| table == "kuru_schema") {
            super::migrations::validate_backup_revision(connection).await?;
        }
        // DOLT_HASHOF_DB HEAD reads DSess's checked selected commit root;
        // unlike AS OF it does not resolve the repository head ref. Its
        // other ref argument path accepts named refs, not commit hashes.
        if read_roots.insert(root(connection, "HEAD").await?) {
            read_root(connection, &reference.head, &head_tables, cancellation).await?;
        }
        if let Some(working) = &mut reference.working {
            select(connection, &reference.name, &reference.head).await?;
            working.working_root = root(connection, "WORKING").await?;
            working.staged_root = root(connection, "STAGED").await?;
            // Failed private migration stages intentionally retain partial
            // DDL. Main's existing classifier above proves their exact status;
            // all other application working sets keep committed schema and
            // receipt authority, while retaining arbitrary changed row data.
            let reserved = super::migrations::backup_reserved_branch(&reference.name);
            for selector in ["WORKING", "STAGED"] {
                let selected = tables(connection, selector).await?;
                if !reserved {
                    for table in &head_tables {
                        ensure!(
                            selected.contains(table),
                            "native {:?} ref {ref_index} {selector} removes committed table {table:?}",
                            reference.kind
                        );
                        ensure!(
                            schema(connection, table, &reference.head).await?
                                == schema(connection, table, selector).await?,
                            "native working set changes committed table schema"
                        );
                    }
                    for table in [
                        "kuru_schema",
                        "kuru_migrations",
                        "kuru_migration_publications",
                    ] {
                        ensure!(
                            head_tables.iter().any(|name| name == table)
                                == selected.iter().any(|name| name == table),
                            "native working set adds or removes schema or receipt authority"
                        );
                        if head_tables.iter().any(|name| name == table) {
                            ensure!(
                                fingerprints(connection, table, &reference.head).await?
                                    == fingerprints(connection, table, selector).await?,
                                "native working set changes schema or receipt authority"
                            );
                        }
                    }
                }
                if read_roots.insert(root(connection, selector).await?) {
                    read_root(connection, selector, &selected, cancellation).await?;
                }
            }
        }
    }
    Ok(())
}

async fn select(
    connection: &mut MySqlConnection,
    reference: &str,
    expected_head: &str,
) -> Result<()> {
    let database = format!("kuru/{reference}");
    connection
        .execute(sqlx::AssertSqlSafe(format!(
            "USE {}",
            identifier(&database)?
        )))
        .await?;
    let found: Option<String> = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut *connection)
        .await?;
    ensure!(
        found.as_deref() == Some(database.as_str()),
        "native image reader selected a different revision"
    );
    let resolved: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
        .fetch_one(&mut *connection)
        .await?;
    ensure!(
        resolved == expected_head,
        "native image reader resolved a different committed head"
    );
    Ok(())
}

async fn root(connection: &mut MySqlConnection, selector: &str) -> Result<String> {
    let root: String = sqlx::query_scalar("SELECT DOLT_HASHOF_DB(?)")
        .bind(selector)
        .fetch_one(connection)
        .await?;
    ensure!(
        crate::backup::noms_hash(&root),
        "native image root is malformed"
    );
    Ok(root)
}

fn identifier(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty() && value.len() <= 8192 && !value.chars().any(char::is_control),
        "native image SQL identifier is malformed"
    );
    Ok(format!("`{}`", value.replace('`', "``")))
}

fn selector(value: &str) -> Result<&str> {
    ensure!(
        matches!(value, "WORKING" | "STAGED") || crate::backup::noms_hash(value),
        "native image selector is not a fixed working root or captured commit"
    );
    Ok(value)
}

async fn tables(connection: &mut MySqlConnection, selected: &str) -> Result<Vec<String>> {
    let query = sqlx::AssertSqlSafe(format!("SHOW TABLES AS OF '{}'", selector(selected)?));
    sqlx::query_scalar(query)
        .fetch_all(connection)
        .await
        .map_err(Into::into)
}

async fn schema(connection: &mut MySqlConnection, table: &str, selected: &str) -> Result<String> {
    let query = sqlx::AssertSqlSafe(format!(
        "SHOW CREATE TABLE {} AS OF '{}'",
        identifier(table)?,
        selector(selected)?
    ));
    let row = sqlx::query(query).fetch_one(connection).await?;
    Ok(row.try_get(1)?)
}

async fn projection(
    connection: &mut MySqlConnection,
    table: &str,
    selected: &str,
) -> Result<String> {
    let source = format!("{} AS OF '{}'", identifier(table)?, selector(selected)?);
    let description = connection
        .prepare(sqlx::AssertSqlSafe(format!("SELECT * FROM {source} LIMIT 0")).into_sql_str())
        .await?;
    let columns = description
        .columns()
        .iter()
        .map(|column| {
            identifier(column.name()).map(|name| format!("SHA2(CAST({name} AS BINARY), 256)"))
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(!columns.is_empty(), "native table has no readable columns");
    Ok(format!("SELECT {} FROM {source}", columns.join(", ")))
}

async fn read_root(
    connection: &mut MySqlConnection,
    selected: &str,
    tables: &[String],
    cancellation: &BackupCancellation,
) -> Result<()> {
    for table in tables {
        let query = projection(connection, table, selected).await?;
        let mut rows = sqlx::query(sqlx::AssertSqlSafe(query)).fetch(&mut *connection);
        while let Some(row) = rows.try_next().await? {
            ensure!(!cancellation.is_cancelled(), "backup validation cancelled");
            // Server-side digests force native values to be read without
            // retaining a private body or imposing a new scalar/history cap.
            for column in 0..row.len() {
                let hash: Option<String> = row.try_get(column)?;
                ensure!(
                    hash.is_none_or(|hash| hash.len() == 64
                        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())),
                    "native table value was not read completely"
                );
            }
        }
    }
    Ok(())
}

async fn fingerprints(
    connection: &mut MySqlConnection,
    table: &str,
    selected: &str,
) -> Result<Vec<Vec<Option<String>>>> {
    let query = projection(connection, table, selected).await?;
    let mut rows = sqlx::query(sqlx::AssertSqlSafe(query)).fetch(connection);
    let mut values = Vec::new();
    while let Some(row) = rows.try_next().await? {
        ensure!(
            values.len() < 65,
            "schema or receipt authority exceeds its released bound"
        );
        values.push(
            (0..row.len())
                .map(|column| row.try_get(column))
                .collect::<Result<_, _>>()?,
        );
    }
    values.sort_unstable();
    Ok(values)
}
