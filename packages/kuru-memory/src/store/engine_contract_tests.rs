//! Engine contract tests for store creation from a pre-migrated template
//! (cospec change `memory-engine-contract-tests`).
//!
//! They establish, against the pinned engine, the Dolt behaviours the store
//! creation design (`tmp/roadmap/store-creation-design-2026-09-29.md`,
//! sections 2-7 and 13, spike items S1-S7, plus S8) depends on. The pinned
//! engine is Dolt 2.3.5, upstream commit
//! `ad65af6cc937d10fa3c88e2041fed4325968b581`
//! (`packages/kuru-memory/support/dolt-assets.json`); source links below are
//! at that commit. Every test runs the real engine through the existing
//! supervisor, server and pool code and isolated fixtures. They change no
//! product behaviour. They are contract tests: each assertion pins what the
//! engine was observed to do, and where the design's assumption did not hold
//! the test asserts the observed behaviour and its doc comment names what the
//! design must do instead.
//!
//! Test-only fixture rules: a copied store's `identity.json` carries the
//! source's instance and scope with new secrets, because product bootstrap
//! has no adoption step yet; adoption is performed by the test itself on one
//! connection, exactly as design section 2.1 step 7 describes. Every
//! `AS OF` revision is checked as a 32-character `[0-9a-v]` hash before it is
//! formatted into SQL (Dolt's `AS OF` takes no bind parameter).
//!
//! S8 inventory: the inline checks of `classify_historical_attempts_in`
//! (`store/migrations.rs`), and the main-pool equivalent each was tested
//! against (see [`record_classification_checks_have_main_pool_equivalents`]).
//! "Same answer" is asserted on a migrated store and, where the branch pool
//! can no longer open, on an adopted copy:
//!
//! | Inline check (branch or commit pool) | Main-pool equivalent tested | Same answer |
//! |---|---|---|
//! | head: `DOLT_HASHOF('HEAD')` on the branch pool | `dolt_branches.hash` | yes |
//! | head schema version: `kuru_schema AS OF 'HEAD'` | `kuru_schema AS OF '<head>'` | yes |
//! | dirty count: `dolt_status` on the branch pool | `dolt_branches.dirty`; `` `kuru/<branch>`.dolt_status `` | yes |
//! | clean arm, head: full `validate_version_with` on the branch pool | version `AS OF '<head>'`; `SHOW TABLES` / `SHOW CREATE TABLE ... AS OF '<head>'` | definitions yes; `information_schema` **no** (0 columns for `kuru/<branch>` and `kuru/<head>` from main) |
//! | clean arm, receipt: `kuru_migrations` on the branch pool | `kuru_migrations AS OF '<head>'` | yes |
//! | clean arm, sole parent: `dolt_commit_ancestors` on the branch pool | `dolt_commit_ancestors` on main | yes |
//! | clean arm, parent: full validation on a pool at the parent | version `AS OF '<parent>'`; `SHOW TABLES` / `SHOW CREATE TABLE ... AS OF '<parent>'` | definitions yes; `information_schema` **no** (0 columns for `kuru/<parent>` from main) |
//! | clean arm: head and parent in main's `dolt_log` | unchanged: it already runs on main | yes |
//! | dirty arm, head version: `kuru_schema AS OF 'HEAD'` | `kuru_schema AS OF '<head>'` | yes |
//! | dirty arm, head: full validation on a pool at the head commit | as the clean arm's head row | definitions yes; `information_schema` **no** |
//! | dirty arm, `retained_failed_shape`: `(table_name, staged, status)` of `dolt_status` on the branch pool | `` `kuru/<branch>`.dolt_status `` | yes (synthetic working set) |
//! | dirty arm: head in main's `dolt_log` | unchanged: it already runs on main | yes |
//! | "names active main only when current" | `dolt_branches.hash` against main's head | yes |
//!
//! Full schema validation, at the head as at the parent, is the one check
//! with no drop-in main-pool form: it reads `information_schema` of
//! `DATABASE()`, which main's `information_schema` does not describe for a
//! revision database. The dirty arm is exercised on a synthetic working set
//! (an unstaged new table and a staged modification), not a genuinely failed
//! migration attempt: producing one needs the process-loss fixture of
//! `recovery_tests.rs`, and the queries compared are the same either way.
use super::*;
use crate::test_support::{
    TempDir,
    engine_contract::{self as contract, Capture, CrossOs, Entry, Hit, Needle},
};
use serde_json::json;
use sqlx::{
    Executor,
    mysql::{MySqlConnectOptions, MySqlDatabaseError, MySqlSslMode},
};
use std::{ffi::OsStr, future::Future};

const MIGRATION_PREFIX: &str = "kuru_migration_";
/// Copies and captures have no lease until served, so the fixture guard reads
/// their repositories (`data/kuru/.dolt/stats/.dolt/noms/oldgen`) in full.
const DEPTH: usize = 16;

fn fixture_root() -> Result<TempDir> {
    Ok(TempDir::new("kuru-contract-", None)?.with_depth_budget(DEPTH))
}

fn scope(digit: char) -> String {
    format!("project/{}", digit.to_string().repeat(64))
}

async fn bounded<T>(
    label: &str,
    future: impl Future<Output = std::result::Result<T, sqlx::Error>>,
) -> Result<T> {
    tokio::time::timeout(QUERY_TIMEOUT, future)
        .await
        .with_context(|| format!("{label}: deadline {QUERY_TIMEOUT:?} exceeded"))?
        .with_context(|| label.to_owned())
}

/// Dolt's commit hash: 20 bytes as 32 characters of base32 `{0-9,a-v}`.
fn commit(value: String) -> Result<String> {
    ensure!(
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte)),
        "not a Dolt commit hash: {value:?}"
    );
    Ok(value)
}

fn as_of(hash: &str) -> Result<String> {
    Ok(format!("AS OF '{}'", commit(hash.to_owned())?))
}

fn lifecycle_root(data_dir: &Path) -> Option<PathBuf> {
    cfg!(windows).then(|| data_dir.join("memory/lifecycles"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRecord {
    version: u32,
    instance: String,
    project_scope: String,
    password: String,
    reader_password: String,
    initialized: bool,
}

fn read_identity(store: &Path) -> Result<IdentityRecord> {
    Ok(serde_json::from_slice(&files::read_bytes(
        &store.join("identity.json"),
        16 * 1024,
    )?)?)
}

fn write_identity(store: &Path, identity: &IdentityRecord) -> Result<()> {
    files::write(&store.join("identity.json"), &serde_json::to_vec(identity)?)
}

fn new_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Ref {
    hash: String,
    dirty: bool,
}

async fn refs(pool: &MemoryPool) -> Result<BTreeMap<String, Ref>> {
    let rows = bounded(
        "read dolt_branches",
        sqlx::query("SELECT name, hash, dirty FROM dolt_branches ORDER BY name LIMIT 200")
            .fetch_all(pool),
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("name")?,
                Ref {
                    hash: commit(row.try_get("hash")?)?,
                    dirty: row.try_get::<bool, _>("dirty")?,
                },
            ))
        })
        .collect()
}

fn migration_branches(refs: &BTreeMap<String, Ref>) -> Vec<(String, Ref)> {
    refs.iter()
        .filter(|(name, _)| name.starts_with(MIGRATION_PREFIX))
        .map(|(name, reference)| (name.clone(), reference.clone()))
        .collect()
}

/// A migration branch's target version and receipt operation.
fn attempt(name: &str) -> Result<(i32, String)> {
    let rest = name
        .strip_prefix(MIGRATION_PREFIX)
        .and_then(|rest| rest.strip_prefix('v'))
        .context("not a migration branch")?;
    let (version, operation) = rest.split_once('_').context("malformed migration branch")?;
    Ok((
        version.parse()?,
        Uuid::parse_str(operation)?.hyphenated().to_string(),
    ))
}

async fn port(pool: &MemoryPool) -> Result<u16> {
    let port: i64 = bounded(
        "read engine port",
        sqlx::query_scalar("SELECT CAST(@@port AS SIGNED)").fetch_one(pool),
    )
    .await?;
    Ok(u16::try_from(port)?)
}

async fn connect(
    port: u16,
    user: &str,
    password: &str,
) -> Result<std::result::Result<MySqlConnection, sqlx::Error>> {
    let options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(user)
        .password(password)
        .database("kuru")
        .ssl_mode(MySqlSslMode::Disabled);
    tokio::time::timeout(QUERY_TIMEOUT, MySqlConnection::connect_with(&options))
        .await
        .with_context(|| format!("connect as {user}: deadline exceeded"))
}

fn access_denied(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database)
        if database
            .try_downcast_ref::<MySqlDatabaseError>()
            .is_some_and(|error| error.number() == 1045))
}

async fn use_database(connection: &mut MySqlConnection, database: &str) -> Result<()> {
    bounded(
        "switch database",
        connection.execute(sqlx::AssertSqlSafe(format!("USE `{database}`"))),
    )
    .await?;
    Ok(())
}

/// A stopped, fully migrated source store and what it was created with.
struct Stopped {
    data_dir: PathBuf,
    /// The store directory as the fixture named it.
    named: PathBuf,
    /// The canonical store directory, the only spelling the engine saw.
    store: PathBuf,
    scope: String,
    identity: IdentityRecord,
    refs: BTreeMap<String, Ref>,
    hostname: String,
}

async fn cold_open(data_dir: &Path, scope: &str) -> Result<(OpenOptions, MemoryStore)> {
    crate::test_support::warm_runtime_cache().await?;
    let mut options =
        crate::test_support::warmed_open_options(data_dir.to_owned(), scope.to_owned()).await?;
    options.creation = Creation::Cold;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    Ok((options, store))
}

async fn hostname(pool: &MemoryPool) -> Result<String> {
    bounded(
        "read @@hostname",
        sqlx::query_scalar("SELECT @@hostname").fetch_one(pool),
    )
    .await
}

/// The host name as the operating system reports it, for S7. Dolt 2.3.5
/// builds with Go 1.26.2 (`go/go.mod` at the pinned commit), whose
/// `os.Hostname` reads the `uname` node name (falling back to
/// `/proc/sys/kernel/hostname`) on Linux, the `kern.hostname` sysctl on macOS
/// (what `/bin/hostname` prints) and the physical DNS host name on Windows
/// (<https://github.com/golang/go/blob/go1.26.2/src/os/sys_linux.go>,
/// `sys_bsd.go`, `sys_windows.go`). The engine's `@@hostname` defaults to
/// `os.Hostname()` (go-mysql-server `a939809e084d`, Dolt 2.3.5's pin:
/// <https://github.com/dolthub/go-mysql-server/blob/a939809e084d/sql/variables/system_variables.go#L197-L200>,
/// `hostname` at L1031-L1038); S7 asserts the equality on Unix. Without a new dependency,
/// Windows exposes only the NetBIOS `COMPUTERNAME`, which can differ in case
/// and length, so there it is an extra needle rather than an equality.
#[cfg(target_os = "linux")]
async fn os_hostname() -> Result<String> {
    use std::io::Read;
    // A public kernel file (root-owned, world-readable): read it directly,
    // not through `files::read_bytes`, which accepts only private objects.
    let mut text = String::new();
    std::fs::File::open("/proc/sys/kernel/hostname")
        .context("open /proc/sys/kernel/hostname")?
        .take(1024)
        .read_to_string(&mut text)
        .context("read /proc/sys/kernel/hostname")?;
    Ok(text.trim().to_owned())
}

#[cfg(all(unix, not(target_os = "linux")))]
async fn os_hostname() -> Result<String> {
    let child = {
        // Held across child creation only; see `crate::spawn_gate`.
        let _creation = crate::spawn_gate::child_creation().await;
        tokio::process::Command::new("/bin/hostname")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("start /bin/hostname")?
    };
    let output = tokio::time::timeout(QUERY_TIMEOUT, child.wait_with_output())
        .await
        .context("/bin/hostname: deadline exceeded")??;
    ensure!(output.status.success(), "/bin/hostname failed: {output:?}");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(windows)]
async fn os_hostname() -> Result<String> {
    std::env::var("COMPUTERNAME").context("COMPUTERNAME is not set")
}

async fn stop(options: &OpenOptions, store: MemoryStore) -> Result<Stopped> {
    let refs = refs(&store.pool).await?;
    let hostname = hostname(&store.pool).await?;
    store.close().await?;
    let named = project_directory(&options.data_dir, &options.project_scope)?;
    crate::test_support::await_store_quiescence(
        &named,
        lifecycle_root(&options.data_dir).as_deref(),
    )
    .await?;
    let store = fs::canonicalize(&named)?;
    Ok(Stopped {
        data_dir: options.data_dir.clone(),
        identity: read_identity(&store)?,
        named,
        store,
        scope: options.project_scope.clone(),
        refs,
        hostname,
    })
}

async fn cold_source(data_dir: &Path, scope: &str) -> Result<Stopped> {
    let (options, store) = cold_open(data_dir, scope).await?;
    stop(&options, store).await
}

/// Create the private store directory for `scope` under `data_dir`.
fn store_directory(data_dir: &Path, scope: &str) -> Result<(PathBuf, Directory)> {
    let store = project_directory(data_dir, scope)?;
    private_dir(data_dir)?;
    private_dir(store.parent().context("store has no parent")?)?;
    let directory = files::ensure_private_directory(&store)?;
    Ok((fs::canonicalize(&store)?, directory))
}

/// File-copy the source's `data/` only: no `config/`, so no `privileges.db`.
fn copy_data(source: &Stopped, data_dir: &Path) -> Result<(PathBuf, Vec<Entry>)> {
    let (store, directory) = store_directory(data_dir, &source.scope)?;
    let data = directory.create_private_directory(OsStr::new("data"))?;
    let entries = contract::copy_data_tree(&source.store.join("data"), &data)?;
    Ok((store, entries))
}

fn startup_timeout() -> Duration {
    Duration::from_secs(
        OpenOptions::new(PathBuf::new(), String::new())
            .config
            .startup_timeout_secs,
    )
}

async fn start(data_dir: &Path, store: &Path, scope: &str, read_only: bool) -> Result<Server> {
    let binary = crate::test_support::warm_runtime_cache().await?;
    let options = ServerOptions {
        binary,
        directory: store.to_owned(),
        project_scope: scope.to_owned(),
        supervisor: test_supervisor()?,
        timeout: startup_timeout(),
        read_only,
        retained: None,
        lifecycle_root: lifecycle_root(data_dir),
        ticks: None,
    };
    let opened = {
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(options).await
    };
    opened.map_err(|error| {
        let log = files::read_bytes(&store.join("server.log"), 1024 * 1024)
            .map(|bytes| {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                text[text.len().saturating_sub(4096)..].to_owned()
            })
            .unwrap_or_else(|read| format!("server.log unavailable: {read:#}"));
        error.context(format!(
            "start the engine on {}; server.log tail:\n{log}",
            store.display()
        ))
    })
}

/// A served copy of a source's `data/` with its own new secrets.
struct Served {
    data_dir: PathBuf,
    store: PathBuf,
    identity: IdentityRecord,
    server: Server,
}

async fn serve_copy(source: &Stopped, data_dir: &Path) -> Result<Served> {
    let (store, _) = copy_data(source, data_dir)?;
    serve_data(data_dir, store, &source.scope, &source.identity.instance).await
}

async fn serve_data(
    data_dir: &Path,
    store: PathBuf,
    scope: &str,
    instance: &str,
) -> Result<Served> {
    let identity = IdentityRecord {
        version: 1,
        instance: instance.to_owned(),
        project_scope: scope.to_owned(),
        password: new_secret(),
        reader_password: new_secret(),
        initialized: false,
    };
    write_identity(&store, &identity)?;
    let server = start(data_dir, &store, scope, false).await?;
    let identity = read_identity(&store)?;
    ensure!(
        identity.initialized,
        "bootstrap did not publish the identity"
    );
    Ok(Served {
        data_dir: data_dir.to_owned(),
        store,
        identity,
        server,
    })
}

async fn close(server: Server, pools: impl IntoIterator<Item = Arc<MemoryPool>>) -> Result<()> {
    for pool in pools {
        pool.close().await;
    }
    server.close().await
}

/// Design section 2.1 step 7 (iii): on one connection, `USE` the usage
/// branch, guarded `UPDATE`, `DOLT_COMMIT('-am')`, then the same on main.
async fn adopt(
    connection: &mut MySqlConnection,
    scope: &str,
    from: &str,
    to: &str,
) -> Result<BTreeMap<&'static str, String>> {
    let mut commits = BTreeMap::new();
    for (database, reference) in [
        (
            format!("kuru/{}", usage_ledger::BRANCH),
            usage_ledger::BRANCH,
        ),
        ("kuru".to_owned(), "main"),
    ] {
        use_database(connection, &database).await?;
        let active: String = bounded(
            "read active branch",
            sqlx::query_scalar("SELECT active_branch()").fetch_one(&mut *connection),
        )
        .await?;
        ensure!(active == reference, "USE {database} selected {active}");
        let updated = bounded(
            "update the instance row",
            sqlx::query("UPDATE kuru_instance SET instance_id = ? WHERE singleton = 1 AND instance_id = ? AND project_scope = ?")
                .bind(to)
                .bind(from)
                .bind(scope)
                .execute(&mut *connection),
        )
        .await?
        .rows_affected();
        ensure!(
            updated == 1,
            "guarded adoption updated {updated} rows on {reference}"
        );
        let hash: String = bounded(
            "commit the adoption",
            sqlx::query_scalar(
                "CALL DOLT_COMMIT('-am', 'Adopt Kuru memory template', '--author', ?)",
            )
            .bind(AUTHOR)
            .fetch_one(&mut *connection),
        )
        .await?;
        commits.insert(reference, commit(hash)?);
    }
    Ok(commits)
}

/// Adopt a served copy under a new instance, then restart it with that
/// identity published, as a template-born store would be.
async fn adopt_and_restart(
    served: Served,
    scope: &str,
) -> Result<(Served, BTreeMap<&'static str, String>)> {
    let instance = Uuid::new_v4().to_string();
    let adopted = async {
        let main = served.server.pool("main").await?;
        let commits = match main.acquire().await {
            Ok(connection) => {
                let mut connection = connection.detach();
                let commits =
                    adopt(&mut connection, scope, &served.identity.instance, &instance).await;
                let closed = connection.close().await;
                commits.and_then(|commits| Ok(closed.map(|()| commits)?))
            }
            Err(error) => Err(error),
        };
        main.close().await;
        commits
    }
    .await;
    served.server.close().await?;
    let commits = adopted?;
    let identity = IdentityRecord {
        instance,
        initialized: true,
        ..served.identity
    };
    write_identity(&served.store, &identity)?;
    let server = start(&served.data_dir, &served.store, scope, false).await?;
    Ok((
        Served {
            data_dir: served.data_dir,
            store: served.store,
            identity,
            server,
        },
        commits,
    ))
}

async fn instance_as_of(pool: &MemoryPool, hash: &str) -> Result<String> {
    bounded(
        "read the instance row as of a revision",
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT instance_id FROM kuru_instance {} WHERE singleton = 1",
            as_of(hash)?
        )))
        .fetch_one(pool),
    )
    .await
}

async fn sole_parent_on(pool: &MemoryPool, hash: &str) -> Result<Vec<String>> {
    bounded(
        "read commit parents",
        sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index LIMIT 2",
        )
        .bind(hash)
        .fetch_all(pool),
    )
    .await
}

/// S1. With a populated `data/` and no `config/privileges.db`, Dolt creates
/// `root@localhost` from `DOLT_ROOT_PASSWORD` and Kuru's unchanged bootstrap
/// creates `kuru_reader` with a new secret; the source's users do not exist.
///
/// Upstream: root creation is gated only on `privileges.db` being absent,
/// independent of the data directory
/// (<https://github.com/dolthub/dolt/blob/ad65af6cc937d10fa3c88e2041fed4325968b581/go/cmd/dolt/commands/sqlserver/server.go#L499-L553>,
/// `doesPrivilegesDbExist` at L1155); the privileges file lives in the
/// configured `privilege_file`, not `data/`
/// (<https://www.dolthub.com/docs/sql-reference/server/access-management>).
/// Dolt 2.3.5. If this fails, decision (b) must ship `config/` state or run
/// an explicit credential reset before the adoption bootstrap.
#[tokio::test]
async fn dolt_creates_root_from_environment_with_populated_data_and_empty_config() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let source = cold_source(&root.path().join("source"), &scope('1')).await?;
        let copy = root.path().join("copy");
        let (store, entries) = copy_data(&source, &copy)?;
        ensure!(!entries.is_empty(), "copied an empty data tree");
        let mut before = fs::read_dir(&store)?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>>>()?;
        before.sort();
        assert_eq!(before, ["data"], "the copy must start with data/ only");
        let served = serve_data(&copy, store, &source.scope, &source.identity.instance).await?;
        assert!(
            served.store.join("config/privileges.db").is_file(),
            "the engine did not create a fresh privileges database"
        );
        let main = served.server.pool("main").await?;
        let checked = async {
            let port = port(&main).await?;
            let mut root_session = connect(port, "root", &served.identity.password)
                .await?
                .context("the new root secret from the environment was refused")?;
            let mut users = bounded(
                "list engine users",
                sqlx::query_as::<_, (String, String)>(
                    "SELECT user, host FROM mysql.user ORDER BY user, host",
                )
                .fetch_all(&mut root_session),
            )
            .await?;
            users.sort();
            eprintln!("engine contract S1 users after bootstrap: {users:?}");
            // Dolt adds two engine accounts on every start: the ephemeral
            // `__dolt_local_user__` superuser (server.go L68, L582-L618) and
            // the event scheduler's. Neither is carried by the copied data.
            let (engine, kuru): (Vec<_>, Vec<_>) = users.into_iter().partition(|(user, _)| {
                matches!(user.as_str(), "__dolt_local_user__" | "event_scheduler")
            });
            assert_eq!(
                kuru,
                [
                    ("kuru_reader".to_owned(), "localhost".to_owned()),
                    ("root".to_owned(), "localhost".to_owned())
                ]
            );
            assert!(engine.len() <= 2, "{engine:?}");
            root_session.close().await?;
            let mut reader = connect(port, "kuru_reader", &served.identity.reader_password)
                .await?
                .context("the new reader secret was refused")?;
            let version: i32 = bounded(
                "read as the new reader",
                sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
                    .fetch_one(&mut reader),
            )
            .await?;
            assert_eq!(version, migrations::CURRENT_VERSION);
            reader.close().await?;
            for (user, secret) in [
                ("root", &source.identity.password),
                ("kuru_reader", &source.identity.reader_password),
            ] {
                let error = connect(port, user, secret)
                    .await?
                    .err()
                    .with_context(|| format!("the source's {user} secret still authenticates"))?;
                eprintln!("engine contract S1 source {user} secret: {error}");
                assert!(access_denied(&error), "{user}: {error:#}");
            }
            // Bootstrap found the row, so it committed nothing and left main clean.
            assert_eq!(revision(&main).await?, source.refs["main"].hash);
            let changes: i64 = bounded(
                "read main status",
                sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(main.as_ref()),
            )
            .await?;
            assert_eq!(changes, 0, "bootstrap left main dirty");
            Ok(())
        }
        .await;
        close(served.server, [main]).await?;
        checked
    }
    .await;
    root.release(outcome)
}

/// S2. One bootstrap connection commits a guarded `UPDATE` of the instance
/// row on the usage branch and on main; each head then differs from the
/// template's, has the template head as its sole parent, and holds the new
/// identity; a restart with that identity pools both refs, while a retained
/// migration branch, still holding the template identity, cannot be pooled.
///
/// Upstream: `USE db/branch` switches a session's head
/// (<https://www.dolthub.com/docs/sql-reference/version-control/branches#switch-heads-with-the-use-statement>);
/// `DOLT_COMMIT` commits the session's working set on its active branch
/// (<https://www.dolthub.com/docs/sql-reference/version-control/dolt-sql-procedures#dolt_commit>).
/// Dolt 2.3.5. If this fails, adoption needs one pool per ref, which the
/// identity check refuses for the usage branch before its adoption.
#[tokio::test]
async fn bootstrap_connection_commits_identity_on_main_and_usage_branch() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let source = cold_source(&root.path().join("source"), &scope('2')).await?;
        let served = serve_copy(&source, &root.path().join("copy")).await?;
        let template = (
            source.refs["main"].hash.clone(),
            source.refs[usage_ledger::BRANCH].hash.clone(),
        );
        let main = served.server.pool("main").await?;
        let mut connection = main.acquire().await?.detach();
        let instance = Uuid::new_v4().to_string();
        let checked = async {
            let commits = adopt(
                &mut connection,
                &source.scope,
                &source.identity.instance,
                &instance,
            )
            .await?;
            eprintln!("engine contract S2 adoption commits: {commits:?}");
            let heads: Vec<(String, String, bool)> = bounded(
                "read adopted heads",
                sqlx::query_as("SELECT name, hash, dirty FROM dolt_branches WHERE name = 'main' OR name = ? ORDER BY name")
                    .bind(usage_ledger::BRANCH)
                    .fetch_all(&mut connection),
            )
            .await?;
            assert_eq!(heads.len(), 2, "{heads:?}");
            for (name, hash, dirty) in heads {
                let (reference, previous) = if name == "main" {
                    ("main", &template.0)
                } else {
                    (usage_ledger::BRANCH, &template.1)
                };
                assert_eq!(&hash, &commits[reference], "{name} head is not its adoption commit");
                assert_ne!(&hash, previous, "{name} head did not move");
                assert!(!dirty, "{name} is dirty after adoption");
                let parents: Vec<String> = bounded(
                    "read adoption parent",
                    sqlx::query_scalar("SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index")
                        .bind(&hash)
                        .fetch_all(&mut connection),
                )
                .await?;
                assert_eq!(parents, std::slice::from_ref(previous), "{name} adoption parent");
                let adopted: String = bounded(
                    "read adopted identity",
                    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                        "SELECT instance_id FROM kuru_instance {} WHERE singleton = 1",
                        as_of(&hash)?
                    )))
                    .fetch_one(&mut connection),
                )
                .await?;
                assert_eq!(adopted, instance, "{name} does not hold the new identity");
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let closed = connection.close().await;
        close(served.server, [main]).await?;
        checked?;
        closed?;

        // Restart with the adopted identity published, as creation would.
        let identity = IdentityRecord {
            instance: instance.clone(),
            ..served.identity
        };
        write_identity(&served.store, &identity)?;
        let server = start(&served.data_dir, &served.store, &source.scope, false).await?;
        let main = server.pool("main").await?;
        let usage = server.pool(usage_ledger::BRANCH).await?;
        let checked = async {
            for pool in [&main, &usage] {
                let row: String = bounded(
                    "read adopted identity through the product pool",
                    sqlx::query_scalar("SELECT instance_id FROM kuru_instance WHERE singleton = 1")
                        .fetch_one(pool.as_ref()),
                )
                .await?;
                assert_eq!(row, instance);
            }
            let (branch, _) = migration_branches(&source.refs)
                .into_iter()
                .next()
                .context("the source retained no migration branch")?;
            let error = server
                .pool(&branch)
                .await
                .expect_err("a template-era ref must fail the identity check");
            eprintln!("engine contract S2 template-era pool: {error:#}");
            assert!(
                format!("{error:#}").contains("identity mismatch"),
                "{error:#}"
            );
            Ok(())
        }
        .await;
        close(server, [main, usage]).await?;
        checked
    }
    .await;
    root.release(outcome)
}

/// Read-only attach to a live store as `kuru_reader`, through the product
/// server and its identity-checked pool.
async fn reader(store: &MemoryStore) -> Result<(Server, Arc<MemoryPool>)> {
    let directory = store.shared.directory.clone();
    let data_dir = directory
        .parent()
        .and_then(Path::parent)
        .context("store directory has no data directory")?
        .to_owned();
    let server = start(&data_dir, &directory, &store.shared.project_scope, true).await?;
    let pool = server.pool("main").await?;
    Ok((server, pool))
}

async fn dirty_through_main(pool: &MemoryPool, branch: &str) -> Result<(bool, Vec<String>)> {
    let dirty: bool = bounded(
        "read dolt_branches.dirty",
        sqlx::query_scalar("SELECT dirty FROM dolt_branches WHERE name = ?")
            .bind(branch)
            .fetch_one(pool),
    )
    .await?;
    let tables: Vec<String> = bounded(
        "read a revision-qualified dolt_status",
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT table_name FROM `kuru/{branch}`.dolt_status ORDER BY table_name LIMIT 16"
        )))
        .fetch_all(pool),
    )
    .await?;
    Ok((dirty, tables))
}

/// Make `branch` dirty out of band, from a detached root session on main.
async fn make_dirty(pool: &MemoryPool, branch: &str) -> Result<()> {
    let mut connection = pool.acquire().await?.detach();
    let written = async {
        use_database(&mut connection, &format!("kuru/{branch}")).await?;
        bounded(
            "write to the branch working set",
            sqlx::query("INSERT INTO state (`key`, value) VALUES ('engine-contract-dirty', '1')")
                .execute(&mut connection),
        )
        .await?;
        Ok(())
    }
    .await;
    connection.close().await?;
    written
}

/// S3. The dirty state of a retained migration branch is readable from the
/// main pool, as root and as `kuru_reader`, by both `dolt_branches.dirty`
/// and a revision-qualified `` `kuru/<branch>`.dolt_status ``, with no pool
/// on the branch; both reflect an out-of-band change.
///
/// Upstream: `dirty` compares the branch's working, staged and head roots
/// (<https://github.com/dolthub/dolt/blob/ad65af6cc937d10fa3c88e2041fed4325968b581/go/libraries/doltcore/sqle/dtables/branches_table.go#L208>,
/// `isDirty` L348-L389; documented at
/// <https://www.dolthub.com/docs/sql-reference/version-control/dolt-system-tables#dolt_branches>);
/// revision-qualified names are documented for user tables
/// (<https://www.dolthub.com/docs/sql-reference/version-control/branches#use-fully-qualified-references-with-database-revisions>)
/// and are silent on system tables, so that form is established here only.
/// Dolt 2.3.5. The design should use `dolt_branches.dirty` (one batched
/// query); if neither worked, the dirty check would need a branch pool,
/// which the identity check refuses on every template-era branch.
#[tokio::test]
async fn dolt_branches_reports_dirty_for_root_and_reader() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let (reader_server, reader_pool) = match reader(&store).await {
        Ok(opened) => opened,
        Err(error) => {
            store.close().await?;
            return Err(error);
        }
    };
    let checked = async {
        let branches = migration_branches(&refs(&store.pool).await?);
        let (branch, _) = branches.first().context("no retained migration branch")?;
        for (who, pool) in [("root", &store.pool), ("kuru_reader", &reader_pool)] {
            for (name, _) in &branches {
                assert_eq!(
                    dirty_through_main(pool, name).await?,
                    (false, Vec::new()),
                    "{who}: {name} is dirty before the change"
                );
            }
        }
        make_dirty(&store.pool, branch).await?;
        for (who, pool) in [("root", &store.pool), ("kuru_reader", &reader_pool)] {
            let observed = dirty_through_main(pool, branch).await?;
            eprintln!("engine contract S3 {who} after change: {branch} {observed:?}");
            assert_eq!(observed, (true, vec!["state".to_owned()]), "{who}");
            for (name, _) in branches.iter().skip(1) {
                assert_eq!(dirty_through_main(pool, name).await?, (false, Vec::new()));
            }
            assert!(!refs(pool).await?["main"].dirty, "{who}: main became dirty");
        }
        Ok(())
    }
    .await;
    close(reader_server, [reader_pool]).await?;
    store.close().await?;
    checked
}

async fn engine_sessions(pool: &MemoryPool) -> Result<Vec<Option<String>>> {
    bounded(
        "read engine sessions",
        sqlx::query_scalar("SELECT db FROM information_schema.processlist ORDER BY id")
            .fetch_all(pool),
    )
    .await
}

/// S4. From a connection on main, as root and as `kuru_reader`, `AS OF` a
/// retained branch's name, its head hash and its parent commit hash return
/// that revision's schema version and receipt, equal to what a pool on the
/// branch reads, while no engine session is on a migration revision.
///
/// Upstream: `AS OF` accepts a branch, tag or commit hash
/// (<https://www.dolthub.com/docs/sql-reference/version-control/querying-history>);
/// privileges on revision databases are undocumented, so the reader grant
/// (`SELECT ON kuru.*`) covering `AS OF` is established here only. Dolt 2.3.5.
/// If the reader could not use `AS OF`, record checks on read-only opens
/// would need a root connection.
#[tokio::test]
async fn as_of_reads_through_main_pool_open_no_revision_connection() -> Result<()> {
    // The comparison pools each retained branch, which only a cold store's
    // branches allow: a template copy's branches carry the placeholder identity.
    let store = MemoryStore::temporary_cold().await?;
    let (reader_server, reader_pool) = match reader(&store).await {
        Ok(opened) => opened,
        Err(error) => {
            store.close().await?;
            return Err(error);
        }
    };
    let checked = async {
        let branches = migration_branches(&refs(&store.pool).await?);
        ensure!(!branches.is_empty(), "no retained migration branch");
        let mut observed = Vec::new();
        for (who, pool) in [("root", &store.pool), ("kuru_reader", &reader_pool)] {
            for (name, reference) in &branches {
                let (target, operation) = attempt(name)?;
                let parent = sole_parent_on(pool, &reference.hash).await?;
                assert_eq!(parent.len(), 1, "{who}: {name} parents {parent:?}");
                let parent = &parent[0];
                let by_name: i32 = bounded(
                    "read the schema AS OF a branch name",
                    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                        "SELECT version FROM kuru_schema AS OF '{name}' WHERE id = 1"
                    )))
                    .fetch_one(pool.as_ref()),
                )
                .await?;
                let (by_hash, receipt): (i32, String) = bounded(
                    "read the schema and receipt AS OF the head",
                    sqlx::query_as(sqlx::AssertSqlSafe(format!(
                        "SELECT (SELECT version FROM kuru_schema {0} WHERE id = 1), (SELECT operation FROM kuru_migrations {0} WHERE version = {target})",
                        as_of(&reference.hash)?
                    )))
                    .fetch_one(pool.as_ref()),
                )
                .await?;
                let before: i32 = bounded(
                    "read the schema AS OF the parent commit",
                    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                        "SELECT version FROM kuru_schema {} WHERE id = 1",
                        as_of(parent)?
                    )))
                    .fetch_one(pool.as_ref()),
                )
                .await?;
                assert_eq!((by_name, by_hash, before), (target, target, target - 1), "{who}: {name}");
                assert_eq!(receipt, operation, "{who}: {name}");
                observed.push((name.clone(), parent.clone(), target - 1, receipt));
            }
        }
        let sessions = engine_sessions(&store.pool).await?;
        eprintln!("engine contract S4 engine sessions: {sessions:?}");
        assert!(
            sessions
                .iter()
                .flatten()
                .all(|db| !db.starts_with(&format!("kuru/{MIGRATION_PREFIX}"))),
            "an engine session is on a migration revision: {sessions:?}"
        );
        assert!(
            sessions.iter().flatten().any(|db| db == "kuru/main"),
            "the session list does not name databases: {sessions:?}"
        );
        // The same values through a pool on each revision, as today's
        // classifier reads them.
        for (name, parent, before, receipt) in observed.iter().take(branches.len()) {
            let (target, _) = attempt(name)?;
            let branch = store.shared.server.pool(name).await?;
            let read = async {
                let head_version: i32 = bounded(
                    "read the branch schema",
                    sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
                        .fetch_one(branch.as_ref()),
                )
                .await?;
                let branch_receipt: String = bounded(
                    "read the branch receipt",
                    sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = ?")
                        .bind(target)
                        .fetch_one(branch.as_ref()),
                )
                .await?;
                Ok::<_, anyhow::Error>((head_version, branch_receipt))
            }
            .await;
            branch.close().await;
            assert_eq!(read?, (target, receipt.clone()), "{name}");
            let commit_pool = store.shared.server.pool(parent).await?;
            let version = migrations::version(&commit_pool).await;
            commit_pool.close().await;
            assert_eq!(version?, *before, "{name} parent");
        }
        Ok(())
    }
    .await;
    close(reader_server, [reader_pool]).await?;
    store.close().await?;
    checked
}

/// S6. Every commit hash the engine returns (`DOLT_HASHOF`, `DOLT_COMMIT`,
/// `dolt_branches`, `dolt_log`, `dolt_commit_ancestors`) is 32 characters of
/// lowercase base32 `{0-9,a-v}`, and round-trips exactly through the
/// record design's `CHAR(32) CHARACTER SET ascii COLLATE ascii_bin` column,
/// which refuses a longer value.
///
/// Upstream: `ByteLen = 20`, `StringLen = 32`, pattern `^([0-9a-v]{32})$`
/// (<https://github.com/dolthub/dolt/blob/ad65af6cc937d10fa3c88e2041fed4325968b581/go/store/hash/hash.go#L61-L75>),
/// alphabet `0123456789abcdefghijklmnopqrstuv`
/// (<https://github.com/dolthub/dolt/blob/ad65af6cc937d10fa3c88e2041fed4325968b581/go/store/hash/base32.go#L26>).
/// Dolt 2.3.5. If this fails, the record columns must be widened and the
/// `AS OF` hash check (store.rs `context_summary_source_unchanged`) revised.
#[tokio::test]
async fn commit_hash_width_matches_record_columns() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let checked = async {
        let pool = store.pool.as_ref();
        let head = commit(revision(pool).await?)?;
        let width: i64 = bounded(
            "measure the head hash",
            sqlx::query_scalar("SELECT CAST(LENGTH(DOLT_HASHOF('HEAD')) AS SIGNED)").fetch_one(pool),
        )
        .await?;
        assert_eq!(width, 32);
        let committed: String = bounded(
            "commit",
            sqlx::query_scalar("CALL DOLT_COMMIT('--allow-empty', '-m', 'engine contract hash probe', '--author', ?)")
                .bind(AUTHOR)
                .fetch_one(pool),
        )
        .await?;
        let committed = commit(committed)?;
        assert_ne!(committed, head);
        let mut hashes: Vec<String> = bounded(
            "read the log",
            sqlx::query_scalar("SELECT commit_hash FROM dolt_log LIMIT 1000").fetch_all(pool),
        )
        .await?;
        hashes.extend(
            bounded(
                "read ancestors",
                sqlx::query_scalar("SELECT parent_hash FROM dolt_commit_ancestors WHERE parent_hash IS NOT NULL LIMIT 1000")
                    .fetch_all(pool),
            )
            .await?,
        );
        hashes.extend(refs(pool).await?.into_values().map(|reference| reference.hash));
        let count = hashes.len();
        for hash in hashes {
            commit(hash)?;
        }
        eprintln!("engine contract S6: {count} engine hashes are 32 base32 characters, e.g. {committed}");

        let mut connection = pool.acquire().await?.detach();
        let columns = async {
            bounded(
                "create the record column probe",
                sqlx::query("CREATE TEMPORARY TABLE engine_contract_hashes (hash CHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY)")
                    .execute(&mut connection),
            )
            .await?;
            for hash in [&head, &committed] {
                bounded(
                    "store a hash",
                    sqlx::query("INSERT INTO engine_contract_hashes VALUES (?)")
                        .bind(hash)
                        .execute(&mut connection),
                )
                .await?;
            }
            let stored: Vec<String> = bounded(
                "read stored hashes",
                sqlx::query_scalar("SELECT hash FROM engine_contract_hashes ORDER BY hash")
                    .fetch_all(&mut connection),
            )
            .await?;
            let mut expected = vec![head.clone(), committed.clone()];
            expected.sort();
            assert_eq!(stored, expected);
            let longer = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query("INSERT INTO engine_contract_hashes VALUES (?)")
                    .bind(format!("{head}0"))
                    .execute(&mut connection),
            )
            .await
            .context("longer hash insert deadline exceeded")?;
            eprintln!("engine contract S6 33-character insert: {longer:?}");
            assert!(longer.is_err(), "a 33-character value entered CHAR(32)");
            Ok(())
        }
        .await;
        connection.close().await?;
        columns
    }
    .await;
    store.close().await?;
    checked
}

fn scan_needles(label: &str, source: &Stopped, extra: &[(&str, String)]) -> Vec<Needle> {
    let mut needles = Vec::new();
    let mut add = |name: String, text: String| needles.extend(contract::needles(&name, &text));
    add(
        format!("{label} store path"),
        source.store.display().to_string(),
    );
    add(
        format!("{label} named store path"),
        source.named.display().to_string(),
    );
    if let Ok(data_dir) = fs::canonicalize(&source.data_dir) {
        add(
            format!("{label} data directory"),
            data_dir.display().to_string(),
        );
    }
    // A compressed chunk can back-reference part of a repeated string, so
    // each secret is also sought as four 16-character fragments.
    for (name, secret) in [
        ("root secret", &source.identity.password),
        ("reader secret", &source.identity.reader_password),
    ] {
        add(format!("{label} {name}"), secret.clone());
        for (index, fragment) in secret.as_bytes().chunks(16).enumerate() {
            add(
                format!("{label} {name} fragment {index}"),
                String::from_utf8_lossy(fragment).into_owned(),
            );
        }
    }
    add("host name".into(), source.hostname.clone());
    for (name, text) in extra {
        add((*name).to_owned(), text.clone());
    }
    needles
}

/// A random uppercase canary with no repeated 4-byte run, so neither its
/// own text nor the lowercase, digit and hex content around it can give a
/// compressor a back-reference that splits it.
fn canary(kind: &str) -> String {
    loop {
        let letters: String = Uuid::new_v4()
            .as_bytes()
            .iter()
            .chain(Uuid::new_v4().as_bytes())
            .map(|byte| char::from(b'A' + byte % 26))
            .collect();
        let candidate = format!("KURUCONTRACT{kind}{letters}");
        let runs: BTreeSet<&[u8]> = candidate.as_bytes().windows(4).collect();
        if runs.len() == candidate.len() - 3 {
            return candidate;
        }
    }
}

fn report(label: &str, entries: &[Entry], hits: &[Hit]) {
    let files = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::File { bytes, .. } => Some(format!("{} ({bytes} B)", entry.path())),
            Entry::Directory { .. } => None,
        })
        .collect::<Vec<_>>();
    eprintln!(
        "engine contract S7 {label}: {} files: {files:#?}\nhits: {hits:#?}",
        files.len()
    );
}

/// S7. A stopped store's `data/`, and a copy of it after one more served
/// start, hold no absolute path of either store, no host name and no secret
/// of either store in any byte sequence the scan can see. The inventory,
/// including the statistics store under `.dolt/stats`, is printed. The host
/// name needles are the engine's `@@hostname` and the operating system's
/// host name ([`os_hostname`]); both are printed, and on Unix asserted equal.
///
/// Positive controls bound what "can see" means. Chunk records are
/// compressed, so the scan is only evidence for strings that stay literal:
/// a 9-character fragment of the instance UUID (a short column of the
/// identity row) and a repeat-free canary in the `VARBINARY` message
/// namespace must be found; whether the whole UUID and a canary in a
/// `LONGTEXT` value are visible is printed, not asserted. A
/// negative for a secret or path written only inside compressed or
/// out-of-band values is therefore not established by this test.
///
/// Upstream: `repo_state.json` holds head, remotes, backups and branches
/// (<https://github.com/dolthub/dolt/blob/ad65af6cc937d10fa3c88e2041fed4325968b581/go/libraries/doltcore/env/repo_state.go>);
/// storage is content-addressed
/// (<https://www.dolthub.com/blog/2024-10-28-dolt-anatomy/>); stopped-server
/// file copies are the documented backup method
/// (<https://www.dolthub.com/docs/sql-reference/server/backups>). Dolt 2.3.5.
/// If a needle is found, the template producer must refuse those bytes and
/// the capture must be rewritten or produced elsewhere.
#[tokio::test]
async fn stopped_data_tree_holds_no_host_path_or_secret_bytes() -> Result<()> {
    let root = fixture_root()?;
    let container = root
        .path()
        .parent()
        .and_then(Path::file_name)
        .context("fixture root has no container name")?
        .to_string_lossy()
        .into_owned();
    let outcome = async {
        // Uppercase letters share no 4-byte run with the hex, digits and
        // lowercase names around them, so no compressor back-reference can
        // split a canary (a first run whose canary repeated its key's text
        // was not found).
        let inline = canary("INLINE");
        let text = canary("TEXT");
        let (options, store) = cold_open(&root.path().join("source"), &scope('7')).await?;
        let written = async {
            store.append(&inline, "user", "engine contract").await?;
            store
                .put("engine-contract-canary", &json!({ "canary": text }))
                .await
        }
        .await;
        let source = stop(&options, store).await?;
        written?;
        ensure!(
            !source.hostname.is_empty(),
            "the engine reported no host name"
        );
        let os_host = os_hostname().await?;
        eprintln!(
            "engine contract S7 host name needles: engine @@hostname {:?}, operating system {os_host:?}",
            source.hostname
        );
        if cfg!(unix) {
            assert_eq!(
                source.hostname, os_host,
                "@@hostname differs from the operating system's host name"
            );
        }
        // The instance UUID is hex and can repeat a 4-byte run of its own,
        // so the control asks for any 9-character fragment of it; the whole
        // UUID is only observed.
        let mut controls = vec![
            (
                "observed: instance".to_owned(),
                source.identity.instance.clone(),
            ),
            ("control: inline canary".to_owned(), inline.clone()),
            ("observed: LONGTEXT canary".to_owned(), text.clone()),
        ];
        for (index, fragment) in source.identity.instance.as_bytes().chunks(9).enumerate() {
            controls.push((
                format!("control: instance fragment {index}"),
                String::from_utf8_lossy(fragment).into_owned(),
            ));
        }
        let mut needles = scan_needles(
            "source",
            &source,
            &[
                ("fixture container", container.clone()),
                ("operating system host name", os_host.clone()),
            ],
        );
        for (label, value) in &controls {
            needles.extend(contract::needles(label, value));
        }
        let check = |label: &str, hits: &[Hit]| {
            for control in ["control: instance", "control: inline canary"] {
                assert!(
                    hits.iter().any(|hit| hit.label.starts_with(control)),
                    "{label}: positive control {control} was not found by the scan"
                );
            }
            for observed in ["observed: instance", "observed: LONGTEXT canary"] {
                eprintln!(
                    "engine contract S7 {label}: {observed} visible: {}",
                    hits.iter().any(|hit| hit.label.starts_with(observed))
                );
            }
            let leaks: Vec<_> = hits
                .iter()
                .filter(|hit| {
                    !hit.label.starts_with("control:") && !hit.label.starts_with("observed:")
                })
                .collect();
            assert!(
                leaks.is_empty(),
                "{label} holds identifying bytes: {leaks:#?}"
            );
        };
        let (entries, hits) = contract::scan(&source.store.join("data"), &needles)?;
        report("source", &entries, &hits);
        let stats = source.store.join("data/kuru/.dolt/stats").exists();
        eprintln!("engine contract S7 statistics store present: {stats}");
        check("source data tree", &hits);

        let served = serve_copy(&source, &root.path().join("copy")).await?;
        let copy_store = served.store.clone();
        let copy_identity = served.identity.clone();
        served.server.close().await?;
        crate::test_support::await_store_quiescence(
            &copy_store,
            lifecycle_root(&served.data_dir).as_deref(),
        )
        .await?;
        let copy = Stopped {
            data_dir: served.data_dir.clone(),
            named: project_directory(&served.data_dir, &source.scope)?,
            store: copy_store.clone(),
            scope: source.scope.clone(),
            identity: copy_identity,
            refs: BTreeMap::new(),
            hostname: source.hostname.clone(),
        };
        needles.extend(scan_needles("copy", &copy, &[]));
        let (entries, hits) = contract::scan(&copy_store.join("data"), &needles)?;
        report("served copy", &entries, &hits);
        check("served copy data tree", &hits);
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// One retained branch's classification inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Classified {
    name: String,
    head: String,
    head_version: i32,
    dirty: i64,
    receipt: String,
    parent: String,
    parent_version: i32,
    head_in_main: bool,
    parent_in_main: bool,
    names_main: bool,
}

async fn in_main_log(main: &MemoryPool, hash: &str) -> Result<bool> {
    let count: i64 = bounded(
        "look up main's log",
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log WHERE commit_hash = ?")
            .bind(hash)
            .fetch_one(main),
    )
    .await?;
    Ok(count == 1)
}

/// Today's answers, read exactly as `classify_historical_attempts_in` reads
/// them: through a pool on the branch and a pool at its parent commit.
async fn classify_with_branch_pools(
    server: &Server,
    main: &MemoryPool,
    name: &str,
) -> Result<Classified> {
    let (target, _) = attempt(name)?;
    let main_head = revision(main).await?;
    let attempt_pool = server.pool(name).await?;
    let inspected = async {
        let head = revision(&attempt_pool).await?;
        let head_version: i32 = bounded(
            "branch schema",
            sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
                .fetch_one(attempt_pool.as_ref()),
        )
        .await?;
        let dirty: i64 = bounded(
            "branch status",
            sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(attempt_pool.as_ref()),
        )
        .await?;
        let receipt: String = bounded(
            "branch receipt",
            sqlx::query_scalar("SELECT operation FROM kuru_migrations WHERE version = ?")
                .bind(target)
                .fetch_one(attempt_pool.as_ref()),
        )
        .await?;
        // `validate_version_with` at the head: full schema validation on the
        // branch pool, whose `information_schema` queries use `DATABASE()`.
        let validated = migrations::validate_supported(&attempt_pool).await?;
        ensure!(
            validated == head_version,
            "{name}: full validation read version {validated}, AS OF 'HEAD' {head_version}"
        );
        let parents = sole_parent_on(&attempt_pool, &head).await?;
        ensure!(parents.len() == 1, "{name} has parents {parents:?}");
        Ok::<_, anyhow::Error>((head, head_version, dirty, receipt, parents[0].clone()))
    }
    .await;
    attempt_pool.close().await;
    let (head, head_version, dirty, receipt, parent) = inspected?;
    let parent_pool = server.pool(&parent).await?;
    // `validate_commit_version`: full schema validation at the parent.
    let parent_version = migrations::validate_supported(&parent_pool).await;
    parent_pool.close().await;
    Ok(Classified {
        name: name.to_owned(),
        head_in_main: in_main_log(main, &head).await?,
        parent_in_main: in_main_log(main, &parent).await?,
        names_main: head == main_head,
        head,
        head_version,
        dirty,
        receipt,
        parent,
        parent_version: parent_version?,
    })
}

/// The same answers through the main pool only.
async fn classify_through_main(main: &MemoryPool, name: &str) -> Result<Classified> {
    let (target, _) = attempt(name)?;
    let all = refs(main).await?;
    let reference = all
        .get(name)
        .with_context(|| format!("{name} is missing"))?;
    let head = reference.hash.clone();
    let (head_version, receipt): (i32, String) = bounded(
        "schema and receipt AS OF the head",
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT (SELECT version FROM kuru_schema {0} WHERE id = 1), (SELECT operation FROM kuru_migrations {0} WHERE version = {target})",
            as_of(&head)?
        )))
        .fetch_one(main),
    )
    .await?;
    let (dirty_flag, tables) = dirty_through_main(main, name).await?;
    ensure!(
        dirty_flag == !tables.is_empty(),
        "{name}: dolt_branches.dirty {dirty_flag} disagrees with dolt_status {tables:?}"
    );
    let parents = sole_parent_on(main, &head).await?;
    ensure!(
        parents.len() == 1,
        "{name} has parents {parents:?} through main"
    );
    let parent = parents[0].clone();
    let parent_version: i32 = bounded(
        "schema AS OF the parent",
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM kuru_schema {} WHERE id = 1",
            as_of(&parent)?
        )))
        .fetch_one(main),
    )
    .await?;
    Ok(Classified {
        name: name.to_owned(),
        head_in_main: in_main_log(main, &head).await?,
        parent_in_main: in_main_log(main, &parent).await?,
        names_main: head == all["main"].hash,
        head,
        head_version,
        dirty: i64::try_from(tables.len())?,
        receipt,
        parent,
        parent_version,
    })
}

/// A revision's table definitions: `SHOW TABLES` and `SHOW CREATE TABLE`,
/// either on a pool at the revision (`as_of` empty) or `AS OF` from main.
async fn schema_text(pool: &MemoryPool, as_of: &str) -> Result<Vec<(String, String)>> {
    let tables: Vec<String> = bounded(
        "list tables",
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SHOW TABLES {as_of}"))).fetch_all(pool),
    )
    .await?;
    let mut definitions = Vec::new();
    for table in tables {
        ensure!(
            table
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "unexpected table name {table:?}"
        );
        let row = bounded(
            "show a table definition",
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SHOW CREATE TABLE `{table}` {as_of}"
            )))
            .fetch_one(pool),
        )
        .await?;
        definitions.push((table, row.try_get::<String, _>(1)?));
    }
    definitions.sort();
    Ok(definitions)
}

/// One revision's schema read through a pool opened on `target` (a branch
/// or a commit hash) and from main: the table definitions must be equal
/// (`AS OF hash` from main), and the `information_schema` column counts are
/// returned for the pool and, from main, for `kuru/<target>` and
/// `kuru/<hash>`.
async fn revision_schema(
    server: &Server,
    main: &MemoryPool,
    target: &str,
    hash: &str,
) -> Result<(i64, BTreeMap<String, i64>)> {
    let pool = server.pool(target).await?;
    let at_pool = async {
        Ok::<_, anyhow::Error>((
            schema_text(&pool, "").await?,
            column_count(&pool, None).await?,
        ))
    }
    .await;
    pool.close().await;
    let (definitions, columns) = at_pool?;
    let through_main = schema_text(main, &as_of(hash)?).await?;
    assert_eq!(
        through_main, definitions,
        "{target}: table definitions differ AS OF {hash}"
    );
    let mut from_main = BTreeMap::new();
    for database in [target, hash] {
        let database = format!("kuru/{database}");
        let count = column_count(main, Some(&database)).await?;
        from_main.insert(database, count);
    }
    Ok((columns, from_main))
}

/// A branch's working-set inventory, as `retained_failed_shape` reads it:
/// through a pool on the branch (`branch` `None`) or, from main, through the
/// revision-qualified `` `kuru/<branch>`.dolt_status ``.
async fn status_rows(
    pool: &MemoryPool,
    branch: Option<&str>,
) -> Result<Vec<(String, i64, String)>> {
    let table = match branch {
        Some(branch) => format!("`kuru/{branch}`.dolt_status"),
        None => "dolt_status".to_owned(),
    };
    let rows = bounded(
        "read a working-set inventory",
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT table_name, staged, status FROM {table} ORDER BY BINARY table_name, staged, BINARY status LIMIT 16"
        )))
        .fetch_all(pool),
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

/// Run `statements` on `branch`'s working set from a detached root session
/// on main.
async fn on_branch(pool: &MemoryPool, branch: &str, statements: &[&str]) -> Result<()> {
    let mut connection = pool.acquire().await?.detach();
    let written = async {
        use_database(&mut connection, &format!("kuru/{branch}")).await?;
        for statement in statements {
            bounded(
                statement,
                sqlx::query(sqlx::AssertSqlSafe(*statement)).fetch_all(&mut connection),
            )
            .await?;
        }
        Ok(())
    }
    .await;
    connection.close().await?;
    written
}

/// The dirty arm of `classify_historical_attempts_in` on a dirtied retained
/// branch: the head, `kuru_schema AS OF 'HEAD'` and the working-set
/// inventory through a pool on the branch, against `dolt_branches`,
/// `AS OF '<head>'` and the revision-qualified `dolt_status` from main.
async fn dirty_arm_matches(server: &Server, main: &MemoryPool, name: &str) -> Result<()> {
    let attempt_pool = server.pool(name).await?;
    let today = async {
        let head = revision(&attempt_pool).await?;
        let version: i32 = bounded(
            "dirty branch schema",
            sqlx::query_scalar("SELECT version FROM kuru_schema AS OF 'HEAD' WHERE id = 1")
                .fetch_one(attempt_pool.as_ref()),
        )
        .await?;
        Ok::<_, anyhow::Error>((head, version, status_rows(&attempt_pool, None).await?))
    }
    .await;
    attempt_pool.close().await;
    let today = today?;
    let reference = refs(main)
        .await?
        .remove(name)
        .context("dirtied branch is missing")?;
    let version: i32 = bounded(
        "dirty branch schema AS OF its head",
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM kuru_schema {} WHERE id = 1",
            as_of(&reference.hash)?
        )))
        .fetch_one(main),
    )
    .await?;
    let through_main = (
        reference.hash.clone(),
        version,
        status_rows(main, Some(name)).await?,
    );
    eprintln!("engine contract S8 dirty arm {name}: branch pool {today:?}, main {through_main:?}");
    assert!(reference.dirty, "{name}: dolt_branches.dirty is false");
    assert_eq!(
        through_main, today,
        "{name}: dirty-arm answers differ from the branch pool"
    );
    Ok(())
}

async fn column_count(pool: &MemoryPool, schema: Option<&str>) -> Result<i64> {
    let query = match schema {
        Some(schema) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = ?",
        )
        .bind(schema.to_owned()),
        None => sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = DATABASE()",
        ),
    };
    bounded("count schema columns", query.fetch_one(pool)).await
}

/// S8. Every inline check of `classify_historical_attempts_in` except full
/// schema validation, at the head and at the parent, has a main-pool
/// equivalent that returns the same answer, on the migrated source and,
/// unchanged, on an adopted copy whose main instance row differs from every
/// retained branch's; there today's classifier fails closed on identity.
/// Full validation: `AS OF` gives the version and `SHOW TABLES` /
/// `SHOW CREATE TABLE ... AS OF` give the same definitions as a pool on the
/// head or at the parent, but main's `information_schema` returns no columns
/// for the revision databases `kuru/<branch>`, `kuru/<head>` and
/// `kuru/<parent>`, which today's validator queries through `DATABASE()`.
/// The dirty arm (head version, full validation at the head and the
/// working-set inventory `retained_failed_shape` reads) is compared on a
/// synthetic dirty working set on the source, then restored; a genuinely
/// failed attempt needs the process-loss fixture of `recovery_tests.rs` and
/// is out of scope. A dirtied branch and a moved ref change the main-pool
/// answers, so a record check built on them fails closed.
///
/// Upstream: system tables and `AS OF` as cited on S3 and S4;
/// `SHOW CREATE TABLE ... AS OF` and `SHOW TABLES AS OF`
/// (<https://www.dolthub.com/docs/sql-reference/version-control/querying-history>).
/// Dolt 2.3.5. Where an equivalent is missing, the record design must bind
/// that fact at publication time instead of re-deriving it on open, or
/// compare `SHOW CREATE TABLE ... AS OF` output with the expected definitions.
#[tokio::test]
async fn record_classification_checks_have_main_pool_equivalents() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let (options, store) = cold_open(&root.path().join("source"), &scope('8')).await?;
        let compared = async {
            let server = &store.shared.server;
            migrations::validate_active(&store.pool).await?;
            let branches = migration_branches(&refs(&store.pool).await?);
            ensure!(!branches.is_empty(), "no retained migration branch");
            let mut baseline = Vec::new();
            for (name, _) in &branches {
                let today = classify_with_branch_pools(server, &store.pool, name).await?;
                let main = classify_through_main(&store.pool, name).await?;
                assert_eq!(main, today, "{name}: main-pool answers differ from branch pools");
                let (target, operation) = attempt(name)?;
                assert_eq!(
                    (today.head_version, today.parent_version, today.dirty, today.receipt.as_str()),
                    (target, target - 1, 0, operation.as_str())
                );
                assert!(today.head_in_main && today.parent_in_main, "{today:?}");

                for (role, target, hash) in [
                    ("head", name.as_str(), today.head.as_str()),
                    ("parent", today.parent.as_str(), today.parent.as_str()),
                ] {
                    let (pool_columns, from_main) =
                        revision_schema(server, &store.pool, target, hash).await?;
                    eprintln!(
                        "engine contract S8 {name} {role}: information_schema columns at a pool on it {pool_columns}, through main {from_main:?}"
                    );
                    // Observed at the head and at the parent alike: main's
                    // information_schema describes neither the branch nor the
                    // commit revision database, so today's full validation
                    // (information_schema WHERE table_schema = DATABASE()) has
                    // no drop-in main-pool form; the definitions AS OF do.
                    assert!(pool_columns > 0, "{name} {role}: no columns at its pool");
                    assert!(
                        from_main.values().all(|count| *count == 0),
                        "{name} {role}: main's information_schema now describes a revision: {from_main:?}"
                    );
                }
                baseline.push(today);
            }
            // The dirty arm, on one branch dirtied like a failed attempt (a
            // new unstaged table and a staged change), then restored.
            let (dirtied, _) = &branches[0];
            on_branch(
                &store.pool,
                dirtied,
                &[
                    "CREATE TABLE engine_contract_probe (id INT PRIMARY KEY)",
                    "INSERT INTO state (`key`, value) VALUES ('engine-contract-dirty', '1')",
                    "CALL DOLT_ADD('state')",
                ],
            )
            .await?;
            dirty_arm_matches(server, &store.pool, dirtied).await?;
            on_branch(
                &store.pool,
                dirtied,
                &[
                    "DROP TABLE engine_contract_probe",
                    "CALL DOLT_RESET('--hard')",
                ],
            )
            .await?;
            assert_eq!(
                migration_branches(&refs(&store.pool).await?),
                branches,
                "the dirty-arm probe was not restored"
            );
            Ok(baseline)
        }
        .await;
        let source = stop(&options, store).await?;
        let baseline = compared?;

        let served = serve_copy(&source, &root.path().join("copy")).await?;
        let (adopted, commits) = adopt_and_restart(served, &source.scope).await?;
        let main = adopted.server.pool("main").await?;
        let checked = async {
            // The branch-pool path still refuses these refs: opening a pool on
            // a retained branch checks its instance identity against main's,
            // and the adopted copy's branches predate adoption.
            let branch_name = baseline
                .first()
                .map(|today| today.name.clone())
                .context("no retained migration branch in baseline")?;
            let error = classify_with_branch_pools(&adopted.server, &main, &branch_name)
                .await
                .expect_err("branch-pool classification must fail on template-era refs");
            eprintln!("engine contract S8 branch-pool classifier on an adopted copy: {error:#}");
            assert!(format!("{error:#}").contains("identity mismatch"), "{error:#}");
            // The product's main-pool classifier opens no branch pool and so
            // is unaffected by the identity mismatch: it succeeds.
            migrations::validate_active(&main)
                .await
                .context("main-pool classification must succeed on template-era refs")?;
            for today in &baseline {
                let mut expected = today.clone();
                // Main moved to its adoption commit; no retained branch names it.
                expected.names_main = false;
                let adopted_view = classify_through_main(&main, &today.name).await?;
                assert_eq!(adopted_view, expected, "{}: adopted main-pool answers", today.name);
                assert_ne!(instance_as_of(&main, &today.head).await?, adopted.identity.instance);
            }
            assert_eq!(
                instance_as_of(&main, &commits["main"]).await?,
                adopted.identity.instance
            );

            // Fail closed: a dirtied branch and a moved ref are visible.
            let dirtied = &baseline[0];
            make_dirty(&main, &dirtied.name).await?;
            let after = classify_through_main(&main, &dirtied.name).await?;
            assert_eq!(after.dirty, 1, "{after:?}");
            if let Some(moved) = baseline.get(1) {
                bounded(
                    "move a retained ref",
                    sqlx::query("CALL DOLT_BRANCH('-f', ?, ?)")
                        .bind(&moved.name)
                        .bind(&commits["main"])
                        .fetch_all(main.as_ref()),
                )
                .await?;
                let after = classify_through_main(&main, &moved.name).await?;
                eprintln!("engine contract S8 moved ref through main: {after:?}");
                assert_ne!(after.head, moved.head);
                assert_ne!(after.parent, moved.parent);
                assert!(after.names_main);
            }
            Ok(())
        }
        .await;
        close(adopted.server, [main]).await?;
        checked
    }
    .await;
    root.release(outcome)
}

/// Serve a capture's data as a new store, validate it with the product
/// validator, adopt it on one connection and pool both adopted refs.
async fn consume_and_adopt(root: &Path, capture: &Path) -> Result<Capture> {
    let data_dir = root.join("consumer");
    let record = contract::read_capture_record(capture)?;
    let (store, directory) = store_directory(&data_dir, &record.project_scope)?;
    contract::read_capture_tree(capture, &record, &directory)?;
    let served = serve_data(&data_dir, store, &record.project_scope, &record.instance).await?;
    let main = served.server.pool("main").await?;
    let checked = async {
        assert_eq!(migrations::version(&main).await?, record.main_version);
        let heads = refs(&main).await?;
        assert_eq!(heads["main"].hash, record.main_head);
        assert_eq!(heads[usage_ledger::BRANCH].hash, record.usage_head);
        migrations::validate_active(&main).await?;
        Ok(())
    }
    .await;
    main.close().await;
    drop(main);
    if let Err(error) = checked {
        served.server.close().await?;
        return Err(error);
    }
    let (adopted, _) = adopt_and_restart(served, &record.project_scope).await?;
    let pools = async {
        Ok::<_, anyhow::Error>((
            adopted.server.pool("main").await?,
            adopted.server.pool(usage_ledger::BRANCH).await?,
        ))
    }
    .await;
    match pools {
        Ok((main, usage)) => {
            let version = migrations::version(&usage).await;
            close(adopted.server, [main, usage]).await?;
            assert_eq!(version?, record.usage_version);
        }
        Err(error) => {
            adopted.server.close().await?;
            return Err(error);
        }
    }
    Ok(record)
}

/// Capture a stopped cold store as the cross-OS artefact format.
async fn produce_capture(root: &Path, destination: &Path) -> Result<Capture> {
    let (options, store) = cold_open(&root.join("producer"), &scope('5')).await?;
    let versions = async {
        Ok::<_, anyhow::Error>((
            migrations::version(&store.pool).await?,
            store
                .shared
                .usage_pool
                .lock()
                .expect("usage pool lock")
                .clone(),
        ))
    }
    .await;
    let usage_version = match &versions {
        Ok((_, Some(usage))) => Some(migrations::version(usage).await),
        _ => None,
    };
    let source = stop(&options, store).await?;
    let (main_version, _) = versions?;
    let usage_version = usage_version.context("the producer had no usage pool")??;
    contract::write_capture(
        &source.store.join("data"),
        destination,
        Capture {
            format: contract::CAPTURE_FORMAT,
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            engine: provision::DOLT_VERSION.into(),
            project_scope: source.scope.clone(),
            instance: source.identity.instance.clone(),
            main_head: source.refs["main"].hash.clone(),
            usage_head: source.refs[usage_ledger::BRANCH].hash.clone(),
            main_version,
            usage_version,
            entries: Vec::new(),
        },
    )
}

/// S5 (conditional on a later decision). A `data/` tree captured on one OS
/// is served, validated and adopted on another. It runs only when
/// `KURU_ENGINE_CONTRACT_CROSS_OS_CAPTURE` names a capture produced on a
/// different OS; otherwise it prints an explicit not-run result with a fixed
/// token (`[no-capture]` or `[same-os-capture]`) and asserts that the token
/// is the one the environment implies.
/// `KURU_ENGINE_CONTRACT_REQUIRE_CROSS_OS=1` makes not-run a failure, for a
/// CI consuming job. The consumer itself is exercised on this OS by
/// [`same_os_capture_opens_and_adopts_through_the_cross_os_consumer`].
///
/// Upstream is silent on moving a data directory between operating systems;
/// storage is content-addressed and journal-based
/// (<https://www.dolthub.com/blog/2024-10-28-dolt-anatomy/>). If a
/// cross-OS run fails, the template key must include the target and each
/// native job must produce its own template (design section 13.2).
#[tokio::test]
async fn data_tree_captured_on_one_os_opens_and_adopts_on_another() -> Result<()> {
    let require = std::env::var_os(contract::REQUIRE_CROSS_OS).is_some_and(|value| value == "1");
    let provided = std::env::var_os(contract::CROSS_OS_CAPTURE);
    let expected = if provided.is_some() {
        "same-os-capture"
    } else {
        "no-capture"
    };
    match contract::cross_os_plan(provided, require)? {
        CrossOs::NotRun(reason) => {
            eprintln!("engine contract S5 NOT RUN {reason}");
            // The cause follows from the environment: no capture named, or
            // one named that this OS produced. Any other pairing is a defect.
            assert_eq!(reason.token(), expected, "{reason}");
            Ok(())
        }
        CrossOs::Run { capture, record } => {
            eprintln!(
                "engine contract S5 consuming a {} {} capture on {} {}",
                record.os,
                record.arch,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            let root = fixture_root()?;
            let outcome = consume_and_adopt(root.path(), &capture).await;
            root.release(outcome.map(|_| ()))
        }
    }
}

/// The S5 capture format and consumer, end to end on this OS: produce a
/// capture from a stopped cold store, then serve, validate and adopt it.
#[tokio::test]
async fn same_os_capture_opens_and_adopts_through_the_cross_os_consumer() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let capture = root.path().join("capture");
        let produced = produce_capture(root.path(), &capture).await?;
        let consumed = consume_and_adopt(root.path(), &capture).await?;
        assert_eq!(consumed, produced);
        Ok(())
    }
    .await;
    root.release(outcome)
}
