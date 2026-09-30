//! Stages copied from a store template: adoption in the stage engine's
//! bootstrap, its typed verdict, the template shape, and recovery classes R
//! and U (cospec change `memory-template-stage-adoption`).
//!
//! No open creates a template stage yet, so these tests make one by hand, as
//! the template cache and its copy worker will. [`build_template`] builds a
//! template store on one engine under the build identity: initialization,
//! every migration, the usage branch's chain, validation and the template
//! shape with the placeholder row. A stage copies that store's stopped
//! `data/` through the checked copy of the engine contract tests
//! (`test_support::engine_contract::copy_data_tree`) and then receives, last,
//! the identity record a copy writes ([`write_template_stage_identity`]).
//! [`adopt`] runs the stage's one engine start through the stage worker under
//! the project's startup lock, as the creation path will. A template whose
//! bytes a test changes is a [`variant`]: a copy served under the build
//! identity, changed on one session and stopped.
//!
//! Recovery tests count engine starts per stage directory through the engine
//! ledger (`Ledger::starts`), whose key follows the directory through its
//! move to `interrupted/`.
use super::*;
use crate::server::{
    TEMPLATE_INSTANCE, TEMPLATE_SCOPE, TemplateVerdict, compiled_template_key, edit_identity,
    read_identity_view, write_initialized_identity, write_template_build_identity,
    write_template_stage_identity,
};
use crate::test_support::{TempDir, engine_contract as contract, engine_ledger};
use sqlx::{
    Executor,
    mysql::{MySqlConnectOptions, MySqlDatabaseError, MySqlSslMode},
};
use std::ffi::OsStr;

/// Template stores and unopened copies are read in full by the fixture
/// guard (`data/kuru/.dolt/stats/.dolt/noms/oldgen`).
const DEPTH: usize = 16;

fn fixture_root() -> Result<TempDir> {
    Ok(TempDir::new("kuru-template-stage-", None)?.with_depth_budget(DEPTH))
}

fn scope(digit: char) -> String {
    format!("project/{}", digit.to_string().repeat(64))
}

fn lifecycle_root(data_dir: &Path) -> Option<PathBuf> {
    cfg!(windows).then(|| data_dir.join("memory/lifecycles"))
}

fn options(data_dir: &Path, scope: &str) -> Result<OpenOptions> {
    crate::test_support::open_options(data_dir.to_owned(), scope.to_owned())
}

async fn server_options(
    data_dir: &Path,
    directory: &Path,
    scope: &str,
    read_only: bool,
) -> Result<ServerOptions> {
    let options = options(data_dir, scope)?;
    Ok(ServerOptions {
        binary: crate::test_support::warm_runtime_cache().await?,
        directory: directory.to_owned(),
        project_scope: scope.to_owned(),
        supervisor: options
            .supervisor
            .context("fixture options name no supervisor")?,
        timeout: Duration::from_secs(options.config.startup_timeout_secs),
        read_only,
        retained: None,
        lifecycle_root: lifecycle_root(data_dir),
    })
}

/// Start an owned engine on `directory` through the real supervisor.
async fn start(data_dir: &Path, directory: &Path, scope: &str, read_only: bool) -> Result<Server> {
    let options = server_options(data_dir, directory, scope, read_only).await?;
    let _gate = crate::spawn_gate::spawning().await;
    Server::open(options).await
}

async fn bounded<T>(
    label: &str,
    future: impl std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
) -> Result<T> {
    tokio::time::timeout(QUERY_TIMEOUT, future)
        .await
        .with_context(|| format!("{label}: deadline {QUERY_TIMEOUT:?} exceeded"))?
        .with_context(|| label.to_owned())
}

/// Every branch head, by name.
async fn refs<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::MySql>,
) -> Result<BTreeMap<String, String>> {
    let rows: Vec<(String, String)> = bounded(
        "read branch heads",
        sqlx::query_as("SELECT name, hash FROM dolt_branches ORDER BY name LIMIT 200")
            .fetch_all(executor),
    )
    .await?;
    Ok(rows.into_iter().collect())
}

/// The sole parent of `commit`.
async fn parent(pool: &MySqlPool, commit: &str) -> Result<String> {
    let parents: Vec<String> = bounded(
        "read commit parents",
        sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index LIMIT 2",
        )
        .bind(commit)
        .fetch_all(pool),
    )
    .await?;
    let [parent] = parents.as_slice() else {
        bail!("{commit} does not have exactly one parent: {parents:?}");
    };
    Ok(parent.clone())
}

/// A private store directory for `scope` under `data_dir`.
fn store_directory(data_dir: &Path, scope: &str) -> Result<PathBuf> {
    let store = project_directory(data_dir, scope)?;
    private_dir(data_dir)?;
    private_dir(store.parent().context("store has no parent")?)?;
    files::ensure_private_directory(&store)?;
    Ok(fs::canonicalize(&store)?)
}

/// A stopped store template: its store directory, whose `data/` is the
/// template, and the branch heads it was stopped with.
struct Template {
    store: PathBuf,
    refs: BTreeMap<String, String>,
}

impl Template {
    fn head(&self, name: &str) -> Result<&str> {
        self.refs
            .get(name)
            .map(String::as_str)
            .with_context(|| format!("the template has no {name} branch"))
    }
}

/// Build a store template in `data_dir` as the template build will, on one
/// engine: the build identity, initialization, every migration, the usage
/// branch's chain and validation, validation of `main`, and the template
/// shape with the placeholder row.
async fn build_template(data_dir: &Path) -> Result<Template> {
    let store = store_directory(data_dir, TEMPLATE_SCOPE)?;
    write_template_build_identity(&store, compiled_template_key())?;
    let server = start(data_dir, &store, TEMPLATE_SCOPE, false).await?;
    let built = async {
        let main = server.pool("main").await?;
        initialize(&main).await?;
        migrations::upgrade(&server, &main).await?;
        let usage = server.pool(usage_ledger::BRANCH).await?;
        migrations::upgrade_usage(&server, &usage).await?;
        migrations::validate_usage(&usage).await?;
        migrations::validate_active(&main).await?;
        migrations::template_shape::check(&main, migrations::template_shape::Row::Placeholder)
            .await?;
        refs(main.as_ref()).await
    }
    .await;
    let closed = server.close().await;
    let refs = built?;
    closed?;
    let identity = read_identity_view(&store)?;
    ensure!(
        identity.initialized
            && identity.instance == TEMPLATE_INSTANCE
            && identity.project_scope == TEMPLATE_SCOPE
            && identity.template.as_deref() == Some(compiled_template_key()),
        "the template build identity was not published as expected: {identity:?}"
    );
    Ok(Template { store, refs })
}

/// Copy `template`'s `data/` into a new private directory `destination`.
fn copy_data(template: &Template, destination: &Path) -> Result<()> {
    let directory = files::ensure_private_directory(destination)?;
    let data = directory.create_private_directory(OsStr::new("data"))?;
    let entries = contract::copy_data_tree(&template.store.join("data"), &data)?;
    ensure!(!entries.is_empty(), "copied an empty template");
    Ok(())
}

/// A template whose bytes `statements` changed: a copy of `template`
/// served in `data_dir` under the build identity, changed on one session
/// (which starts on `main` and may `USE` another ref), and stopped.
async fn variant(template: &Template, data_dir: &Path, statements: &[&str]) -> Result<Template> {
    let store = store_directory(data_dir, TEMPLATE_SCOPE)?;
    copy_data(template, &store)?;
    write_template_build_identity(&store, compiled_template_key())?;
    let server = start(data_dir, &store, TEMPLATE_SCOPE, false).await?;
    let changed = async {
        let main = server.pool("main").await?;
        let mut connection = bounded("acquire the variant session", main.acquire())
            .await?
            .detach();
        let changed = async {
            for statement in statements {
                bounded(
                    statement,
                    connection.execute(sqlx::AssertSqlSafe((*statement).to_owned())),
                )
                .await?;
            }
            refs(&mut connection).await
        }
        .await;
        let closed = bounded("close the variant session", connection.close()).await;
        let refs = changed?;
        closed?;
        Ok::<_, anyhow::Error>(refs)
    }
    .await;
    let closed = server.close().await;
    let refs = changed?;
    closed?;
    Ok(Template { store, refs })
}

/// The active directory and a new stage name of `scope` under `data_dir`.
fn stage_path(data_dir: &Path, scope: &str) -> Result<(PathBuf, PathBuf)> {
    let active = project_directory(data_dir, scope)?;
    let parent = active.parent().context("store has no parent")?;
    private_dir(data_dir)?;
    private_dir(parent)?;
    let name = active.file_name().context("store has no name")?;
    let stage = parent.join(format!(
        "{}.staging-{}",
        name.to_string_lossy(),
        Uuid::new_v4()
    ));
    Ok((active, stage))
}

/// A copy of `template`'s `data/` in a new stage of `scope`, without an
/// identity record: what an interrupted copy leaves.
fn copy_stage(template: &Template, data_dir: &Path, scope: &str) -> Result<PathBuf> {
    let (_, stage) = stage_path(data_dir, scope)?;
    copy_data(template, &stage)?;
    Ok(fs::canonicalize(&stage)?)
}

/// A complete template stage of `scope`: the copied `data/` and, last, the
/// identity record naming this build's compiled template key.
fn template_stage(template: &Template, data_dir: &Path, scope: &str) -> Result<PathBuf> {
    let stage = copy_stage(template, data_dir, scope)?;
    write_template_stage_identity(&stage, scope, compiled_template_key())?;
    Ok(stage)
}

/// Acquire the project's startup lock as an opener does.
async fn startup_lock(active: &Path) -> Result<File> {
    let parent = active.parent().context("store has no parent")?;
    let locks = parent.join("locks");
    private_dir(&locks)?;
    let directory = files::open_directory(&locks, Privacy::OwnerOnly, NameRetention::Pinned)?;
    let name = active.file_name().context("store has no name")?;
    let lock = acquire_lock(directory.lock_file(name)?, Duration::from_secs(30)).await?;
    directory.verify(name, &lock)?;
    Ok(lock)
}

/// Run the stage worker's adopt job on `stage` under the project's startup
/// lock, which the stage engine holds as its reap guard.
async fn adopt(
    data_dir: &Path,
    scope: &str,
    stage: &Path,
    marker_pause: &mut Option<marker_fixture::ReadyMarkerPause>,
) -> Result<()> {
    let base = server_options(data_dir, stage, scope, false).await?;
    let make_options = |directory: PathBuf, read_only| ServerOptions {
        directory,
        read_only,
        ..base.clone()
    };
    let (active, _) = stage_path(data_dir, scope)?;
    let parent = active.parent().context("store has no parent")?;
    let lifecycle = lifecycle_root(data_dir);
    let worker = stage_worker::StageWorker {
        make_options: &make_options,
        stage,
        parent,
        lifecycle_root: lifecycle.as_deref(),
        timeout: base.timeout,
        project_scope: scope,
        legacy: None,
        migration_hooks: None,
        migrated_stage_pool_delay: None,
    };
    let _gate = crate::spawn_gate::spawning().await;
    let lock = startup_lock(&active).await?;
    let mut progress = ProgressReporter::silent();
    let lock = worker
        .adopt_and_mark(lock, marker_pause, &mut progress)
        .await?;
    drop(lock);
    Ok(())
}

/// Engine starts this process made on the directory `key` names.
fn starts(key: &engine_ledger::Key) -> u64 {
    engine_ledger::with(|ledger| ledger.starts(key))
}

fn ledger_key(directory: &Path) -> Result<engine_ledger::Key> {
    engine_ledger::key(directory)
        .with_context(|| format!("no ledger key for {}", directory.display()))
}

/// Where recovery preserves `stage`.
fn preserved(stage: &Path) -> Result<PathBuf> {
    Ok(stage
        .parent()
        .context("stage has no parent")?
        .join("interrupted")
        .join(stage.file_name().context("stage has no name")?))
}

/// Record the quiescence of a stage recovery preserved without an engine
/// start. Its lease was created by recovery's quiescence wait, and no reap
/// of this process recorded it, so the fixture guard needs this record.
async fn preserved_quiescence(data_dir: &Path, stage: &Path) -> Result<PathBuf> {
    let kept = preserved(stage)?;
    crate::test_support::await_store_quiescence(&kept, lifecycle_root(data_dir).as_deref()).await?;
    Ok(kept)
}

/// An ordinary open of `scope`, which runs recovery first.
async fn open(data_dir: &Path, scope: &str) -> Result<MemoryStore> {
    crate::test_support::spawn_gated_open(options(data_dir, scope)?).await
}

async fn port(pool: &MySqlPool) -> Result<u16> {
    let port: i64 = bounded(
        "read engine port",
        sqlx::query_scalar("SELECT CAST(@@port AS SIGNED)").fetch_one(pool),
    )
    .await?;
    Ok(u16::try_from(port)?)
}

/// Whether `user` with `password` is refused by the engine on `port`.
async fn refused(port: u16, user: &str, password: &str) -> Result<bool> {
    let options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(user)
        .password(password)
        .database("kuru")
        .ssl_mode(MySqlSslMode::Disabled);
    match tokio::time::timeout(QUERY_TIMEOUT, MySqlConnection::connect_with(&options))
        .await
        .context("connect deadline exceeded")?
    {
        Ok(connection) => {
            connection.close().await?;
            Ok(false)
        }
        Err(sqlx::Error::Database(error)) => Ok(error
            .try_downcast_ref::<MySqlDatabaseError>()
            .is_some_and(|error| error.number() == 1045)),
        Err(error) => Err(error.into()),
    }
}

/// The instance row of a ref, read from `main`'s pool by qualified name.
async fn row_on(pool: &MySqlPool, database: &str) -> Result<(String, String)> {
    bounded(
        "read an identity row",
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT instance_id, project_scope FROM `{database}`.kuru_instance WHERE singleton = 1"
        )))
        .fetch_one(pool),
    )
    .await
}

/// Two copies of one template adopt their own instance, scope and
/// credentials; each copy's history begins at its own adoption commits,
/// whose parents are the template's heads; the retained migration branches
/// keep the template's hashes; and neither copy accepts the other's
/// credentials. The initial revision is the `main` adoption head.
#[tokio::test]
async fn adopted_copy_has_own_instance_scope_credentials_and_initial_revision() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let mut copies = Vec::new();
        for digit in ['a', 'b'] {
            let scope = scope(digit);
            let stage = template_stage(&template, &data_dir, &scope)?;
            let pending = read_identity_view(&stage)?;
            ensure!(
                !pending.initialized
                    && pending.template.as_deref() == Some(compiled_template_key())
                    && pending.instance != TEMPLATE_INSTANCE,
                "the stage identity is not a pending template copy: {pending:?}"
            );
            adopt(&data_dir, &scope, &stage, &mut None).await?;
            ensure!(
                stage.join("ready.json").is_file(),
                "the adopt job published no ready marker"
            );
            let store = open(&data_dir, &scope).await?;
            let active = project_directory(&data_dir, &scope)?;
            let identity = read_identity_view(&active)?;
            let activation = read_activation(&active, &scope)?;
            copies.push((scope, store, identity, activation));
        }
        let checked = async {
            for (scope, store, identity, activation) in &copies {
                ensure!(
                    identity.initialized
                        && identity.template.as_deref() == Some(compiled_template_key())
                        && store.service_instance() == identity.instance,
                    "the adopted identity was not published: {identity:?}"
                );
                let heads = refs(store.pool.as_ref()).await?;
                let main = &heads["main"];
                let usage = &heads[usage_ledger::BRANCH];
                ensure!(
                    activation.initial_revision == *main,
                    "the initial revision is not the adoption head of main"
                );
                ensure!(
                    parent(&store.pool, main).await? == template.head("main")?
                        && parent(&store.pool, usage).await?
                            == template.head(usage_ledger::BRANCH)?,
                    "an adoption commit's parent is not the template head"
                );
                for (name, head) in &heads {
                    if name != "main" && name != usage_ledger::BRANCH {
                        ensure!(
                            template.head(name)? == head,
                            "retained branch {name} differs from the template"
                        );
                    }
                }
                for database in ["kuru", crate::server::USAGE_DATABASE] {
                    ensure!(
                        row_on(&store.pool, database).await?
                            == (identity.instance.clone(), scope.clone()),
                        "{database} does not hold the adopted identity"
                    );
                }
                let message: String = bounded(
                    "read the adoption message",
                    sqlx::query_scalar("SELECT message FROM dolt_log LIMIT 1")
                        .fetch_one(store.pool.as_ref()),
                )
                .await?;
                ensure!(message == "Adopt Kuru memory template", "{message}");
            }
            let [(_, first, a, _), (_, second, b, _)] = copies.as_slice() else {
                bail!("expected two copies");
            };
            ensure!(
                a.instance != b.instance
                    && a.secrets.0 != b.secrets.0
                    && a.secrets.1 != b.secrets.1,
                "the copies share an instance or a credential"
            );
            ensure!(
                first.revision().await? != second.revision().await?,
                "the copies share a head"
            );
            for (store, other) in [(first, b), (second, a)] {
                let port = port(&store.pool).await?;
                ensure!(
                    refused(port, "kuru_reader", &other.secrets.1).await?
                        && refused(port, "root", &other.secrets.0).await?,
                    "a copy accepted the other copy's credentials"
                );
            }
            Ok(())
        }
        .await;
        for (_, store, _, _) in copies {
            store.close().await?;
        }
        checked
    }
    .await;
    root.release(outcome)
}

/// Kuru's commit author, for statements that change a variant template.
const VARIANT_AUTHOR: &str = "'Kuru <memory@kuru.local>'";
const FOREIGN_INSTANCE: &str = "11111111-1111-4111-8111-111111111111";

fn foreign_usage_row() -> Vec<String> {
    vec![
        "USE `kuru/kuru_usage_v1`".to_owned(),
        format!("UPDATE kuru_instance SET instance_id = '{FOREIGN_INSTANCE}' WHERE singleton = 1"),
        format!("CALL DOLT_COMMIT('-am', 'foreign usage identity', '--author', {VARIANT_AUTHOR})"),
    ]
}

fn foreign_main_row() -> Vec<String> {
    vec![
        format!("UPDATE kuru_instance SET instance_id = '{FOREIGN_INSTANCE}' WHERE singleton = 1"),
        format!("CALL DOLT_COMMIT('-am', 'foreign main identity', '--author', {VARIANT_AUTHOR})"),
    ]
}

async fn variant_of(
    template: &Template,
    data_dir: &Path,
    statements: &[String],
) -> Result<Template> {
    let statements = statements.iter().map(String::as_str).collect::<Vec<_>>();
    variant(template, data_dir, &statements).await
}

/// The branch heads of a stage's `data/`, read from a copy served in
/// `data_dir` under an initialized identity naming `main`'s row, so that the
/// stage itself never starts again.
async fn inspect(
    stage: &Path,
    data_dir: &Path,
    instance: &str,
    scope: &str,
) -> Result<BTreeMap<String, String>> {
    let store = store_directory(data_dir, scope)?;
    let directory = files::directory(&store)?;
    let data = directory.create_private_directory(OsStr::new("data"))?;
    contract::copy_data_tree(&stage.join("data"), &data)?;
    write_initialized_identity(&store, instance, scope)?;
    let server = start(data_dir, &store, scope, false).await?;
    let heads = async {
        let main = server.pool("main").await?;
        refs(main.as_ref()).await
    }
    .await;
    let closed = server.close().await;
    let heads = heads?;
    closed?;
    Ok(heads)
}

/// Adoption checks this build's compiled key and the compiled placeholder on
/// both refs before it writes: a placeholder row foreign on either ref is a
/// typed verdict, a key other than the compiled one is an ordinary error
/// naming both keys, and in every case neither ref gained a commit and the
/// identity stays uninitialized.
#[tokio::test]
async fn adoption_requires_compiled_key_and_placeholder_on_both_refs() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let cases = [
            (
                "usage",
                variant_of(&template, &root.path().join("usage"), &foreign_usage_row()).await?,
                false,
            ),
            (
                "main",
                variant_of(&template, &root.path().join("main"), &foreign_main_row()).await?,
                false,
            ),
            ("key", template, true),
        ];
        for (index, (label, source, stale_key)) in cases.iter().enumerate() {
            let scope = format!("project/{}", format!("{index:x}").repeat(64));
            let stage = template_stage(source, &data_dir, &scope)?;
            if *stale_key {
                edit_identity(&stage, |identity| {
                    identity.template = Some("a-stale-template-key".to_owned());
                })?;
            }
            let error = adopt(&data_dir, &scope, &stage, &mut None)
                .await
                .err()
                .with_context(|| format!("{label}: adoption succeeded"))?;
            let message = format!("{error:#}");
            if *stale_key {
                ensure!(
                    TemplateVerdict::find(&error).is_none()
                        && message.contains("a-stale-template-key")
                        && message.contains(compiled_template_key()),
                    "{label}: a key mismatch must be an ordinary error naming both keys: {message}"
                );
            } else {
                ensure!(
                    TemplateVerdict::find(&error).is_some()
                        && message.contains(&format!("the {label}"))
                        && message.contains("is not the compiled template placeholder"),
                    "{label}: a foreign placeholder row must be a typed verdict: {message}"
                );
            }
            let identity = read_identity_view(&stage)?;
            ensure!(
                !identity.initialized && !stage.join("ready.json").exists(),
                "{label}: the refused stage was initialized or marked"
            );
            let main_row = if *label == "main" {
                (FOREIGN_INSTANCE, TEMPLATE_SCOPE)
            } else {
                (TEMPLATE_INSTANCE, TEMPLATE_SCOPE)
            };
            let heads = inspect(
                &stage,
                &root.path().join(format!("inspect-{label}")),
                main_row.0,
                main_row.1,
            )
            .await?;
            ensure!(
                heads == source.refs,
                "{label}: a refused adoption changed a ref: {heads:?} != {:?}",
                source.refs
            );
        }
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// A verdict against the template's bytes reaches the caller as the typed
/// verdict; a key mismatch and a SQL error do not. On Unix an in-process
/// supervisor also shows the response frame itself: `TemplateRejected` for
/// a foreign row on either ref, a dirty working set on either ref and a
/// rewrite that changes no row, and `Failed` for a key mismatch, a bootstrap
/// deadline in the middle of adoption and a killed engine.
#[tokio::test]
async fn adoption_verdicts_are_typed_and_engine_failures_are_not() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let foreign = variant_of(
            &template,
            &root.path().join("foreign"),
            &foreign_usage_row(),
        )
        .await?;
        let missing = variant(
            &template,
            &root.path().join("missing"),
            &["CALL DOLT_BRANCH('-D', 'kuru_usage_v1')"],
        )
        .await?;
        let data_dir = root.path().join("projects");

        // Through the spawned supervisor: the client's discriminant.
        let stage = template_stage(&foreign, &data_dir, &scope('1'))?;
        let error = adopt(&data_dir, &scope('1'), &stage, &mut None)
            .await
            .err()
            .context("a foreign row was adopted")?;
        ensure!(
            TemplateVerdict::find(&error).is_some(),
            "a foreign row did not reach the client as a verdict: {error:#}"
        );
        let stage = template_stage(&template, &data_dir, &scope('2'))?;
        edit_identity(&stage, |identity| {
            identity.template = Some("another-build".to_owned());
        })?;
        let error = adopt(&data_dir, &scope('2'), &stage, &mut None)
            .await
            .err()
            .context("a stale key was adopted")?;
        let message = format!("{error:#}");
        ensure!(
            TemplateVerdict::find(&error).is_none()
                && message.contains("another-build")
                && message.contains(compiled_template_key()),
            "a key mismatch was a verdict or did not name both keys: {message}"
        );
        let stage = template_stage(&missing, &data_dir, &scope('3'))?;
        let error = adopt(&data_dir, &scope('3'), &stage, &mut None)
            .await
            .err()
            .context("a template without a usage branch was adopted")?;
        ensure!(
            TemplateVerdict::find(&error).is_none(),
            "a SQL error during adoption was a verdict: {error:#}"
        );

        #[cfg(unix)]
        in_process_responses(root.path(), &template, &foreign).await?;
        Ok(())
    }
    .await;
    root.release(outcome)
}

#[cfg(unix)]
async fn in_process_responses(root: &Path, template: &Template, foreign: &Template) -> Result<()> {
    use crate::server::adoption_fault::{Fault, Outcome, Point, supervise_once};
    let foreign_main =
        variant_of(template, &root.join("foreign-main"), &foreign_main_row()).await?;
    let dirty = variant(
        template,
        &root.join("dirty"),
        &["INSERT INTO state (`key`, value) VALUES ('dirty', 'uncommitted')"],
    )
    .await?;
    // The usage branch's working set, on a branch-qualified session: the
    // dirty-state count must read that ref's status, not `main`'s.
    let dirty_usage = variant(
        template,
        &root.join("dirty-usage"),
        &[
            "USE `kuru/kuru_usage_v1`",
            "CREATE TABLE dirty_usage (id INT PRIMARY KEY)",
        ],
    )
    .await?;
    let data_dir = root.join("in-process");
    /// One in-process start: the stage's source, an optional key to name
    /// instead of the compiled one, the fault, an optional startup timeout,
    /// and the expected response kind and message text.
    struct Case<'a> {
        label: &'a str,
        source: &'a Template,
        key: Option<&'a str>,
        fault: Fault,
        timeout: Option<Duration>,
        rejected: bool,
        expected: &'a str,
    }
    let case = |label, source, fault, rejected, expected| Case {
        label,
        source,
        key: None,
        fault,
        timeout: None,
        rejected,
        expected,
    };
    let cases = vec![
        case(
            "foreign usage row",
            foreign,
            Fault::Nothing,
            true,
            "the usage branch identity row",
        ),
        case(
            "foreign main row",
            &foreign_main,
            Fault::Nothing,
            true,
            "the main identity row",
        ),
        case(
            "dirty main working set",
            &dirty,
            Fault::Nothing,
            true,
            "the main working set holds 1 uncommitted changes",
        ),
        case(
            "dirty usage branch working set",
            &dirty_usage,
            Fault::Nothing,
            true,
            "the usage branch working set holds 1 uncommitted changes",
        ),
        case(
            "rewrite of no row",
            template,
            Fault::MissUsageRewrite,
            true,
            "changed 0 rows",
        ),
        Case {
            key: Some("stale-key"),
            ..case("key mismatch", template, Fault::Nothing, false, "stale-key")
        },
        Case {
            // Long enough that only the stall, never a slow engine start,
            // reaches the deadline.
            timeout: Some(Duration::from_secs(15)),
            ..case(
                "bootstrap deadline",
                template,
                Fault::StallAt(Point::AfterUsageCommit),
                false,
                "deadline exceeded while adopting the store template on the usage branch",
            )
        },
        case(
            "killed engine",
            template,
            Fault::KillEngineAt(Point::AfterUsageCommit, Arc::new(StdMutex::new(None))),
            false,
            "failed while adopting the store template on main",
        ),
    ];
    for (
        index,
        Case {
            label,
            source,
            key,
            fault,
            timeout,
            rejected,
            expected,
        },
    ) in cases.into_iter().enumerate()
    {
        let scope = format!("project/{}", format!("{:x}", index + 4).repeat(64));
        let stage = template_stage(source, &data_dir, &scope)?;
        if let Some(key) = key {
            edit_identity(&stage, |identity| identity.template = Some(key.to_owned()))?;
        }
        let mut options = server_options(&data_dir, &stage, &scope, false).await?;
        if let Some(timeout) = timeout {
            options.timeout = timeout;
        }
        let outcome = supervise_once(&options, fault).await;
        crate::test_support::await_store_quiescence(&stage, None).await?;
        let (was_rejected, message) = match outcome? {
            Outcome::TemplateRejected(message) => (true, message),
            Outcome::Failed(message) => (false, message),
            Outcome::Ready => bail!("{label}: the supervisor adopted the stage"),
        };
        ensure!(
            was_rejected == rejected && message.contains(expected),
            "{label}: expected {} containing {expected:?}, got {} {message}",
            if rejected {
                "TemplateRejected"
            } else {
                "Failed"
            },
            if was_rejected {
                "TemplateRejected"
            } else {
                "Failed"
            },
        );
        ensure!(
            !read_identity_view(&stage)?.initialized,
            "{label}: a failed adoption initialized the stage"
        );
    }
    Ok(())
}

/// Class R: a copy interrupted before its identity record, with or without
/// an interrupted identity write's temporary record, is preserved under
/// `interrupted/` without an engine start, and the open then creates the
/// store afresh.
#[tokio::test]
async fn copy_remnant_without_identity_is_preserved_without_engine_start() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let scope = scope('5');
        let bare = copy_stage(&template, &data_dir, &scope)?;
        let interrupted_write = copy_stage(&template, &data_dir, &scope)?;
        // The directory handle and the record it creates are temporaries
        // dropped before recovery: Windows refuses to move a directory while
        // any descendant is held open.
        files::ensure_private_directory(&interrupted_write.join("staging"))?
            .create_new(OsStr::new(&format!("record-{}.tmp", Uuid::new_v4())))?;
        let stages = [bare, interrupted_write];
        let keys = stages
            .iter()
            .map(|stage| ledger_key(stage))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            keys.iter().all(|key| starts(key) == 0),
            "a copy remnant had an engine start before recovery"
        );
        let store = open(&data_dir, &scope).await?;
        let checked = async {
            for (stage, key) in stages.iter().zip(&keys) {
                ensure!(
                    starts(key) == 0,
                    "recovery started an engine on the copy remnant {}",
                    stage.display()
                );
                let kept = preserved_quiescence(&data_dir, stage).await?;
                ensure!(
                    !stage.exists() && kept.join("data").is_dir(),
                    "the copy remnant {} was not preserved",
                    stage.display()
                );
                ensure!(
                    ledger_key(&kept)? == *key,
                    "the preserved remnant changed identity"
                );
                ensure!(
                    !kept.join("identity.json").exists(),
                    "recovery wrote an identity into a copy remnant"
                );
            }
            ensure!(
                read_identity_view(&project_directory(&data_dir, &scope)?)?
                    .template
                    .is_none(),
                "the store created after recovery is not a cold store"
            );
            Ok(())
        };
        let checked = checked.await;
        store.close().await?;
        checked
    }
    .await;
    root.release(outcome)
}

/// A stage without an identity record that holds anything outside the copy
/// remnant's allowed set is still refused before any engine start, and is
/// left where it is.
#[tokio::test]
async fn unrecognized_stage_without_identity_still_fails_closed() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let foreign_entry = copy_stage(&template, &data_dir, &scope('6'))?;
        files::directory(&foreign_entry)?.create_new(OsStr::new("notes.json"))?;
        let foreign_record = copy_stage(&template, &data_dir, &scope('7'))?;
        files::ensure_private_directory(&foreign_record.join("staging"))?
            .create_new(OsStr::new("endpoint-retired.json"))?;
        let no_data = stage_path(&data_dir, &scope('8'))?.1;
        files::ensure_private_directory(&no_data)?;
        files::ensure_private_directory(&no_data.join("staging"))?;
        for (stage, scope) in [
            (foreign_entry, scope('6')),
            (foreign_record, scope('7')),
            (fs::canonicalize(&no_data)?, scope('8')),
        ] {
            let key = ledger_key(&stage)?;
            let error = open(&data_dir, &scope)
                .await
                .err()
                .with_context(|| format!("{} was accepted", stage.display()))?;
            ensure!(
                format!("{error:#}")
                    .contains("unrecognized interrupted import without server identity"),
                "{error:#}"
            );
            ensure!(
                starts(&key) == 0 && stage.is_dir() && !preserved(&stage)?.exists(),
                "an unrecognized stage was started or moved: {}",
                stage.display()
            );
        }
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// Template stages of `scope` whose adoption an in-process supervisor
/// stopped after the usage branch's commit and after both commits, as a crash
/// there leaves them; their refs are checked on a served copy. Unix only: the
/// fault runs inside an in-process supervisor.
#[cfg(unix)]
async fn mid_adoption_stages(
    root: &Path,
    template: &Template,
    data_dir: &Path,
    scope: &str,
) -> Result<Vec<PathBuf>> {
    use crate::server::adoption_fault::{Fault, Outcome, Point, supervise_once};
    let mut stages = Vec::new();
    for (index, point) in [Point::AfterUsageCommit, Point::AfterMainCommit]
        .into_iter()
        .enumerate()
    {
        let stage = template_stage(template, data_dir, scope)?;
        let options = server_options(data_dir, &stage, scope, false).await?;
        let outcome = supervise_once(&options, Fault::FailAt(point)).await;
        crate::test_support::await_store_quiescence(&stage, None).await?;
        ensure!(
            matches!(outcome?, Outcome::Failed(message) if message.contains("injected")),
            "the adoption fault at {point:?} was not reached"
        );
        let identity = read_identity_view(&stage)?;
        ensure!(
            !identity.initialized,
            "{point:?}: the stage was initialized"
        );
        let (main_instance, main_scope) = match point {
            Point::AfterUsageCommit => (TEMPLATE_INSTANCE, TEMPLATE_SCOPE),
            Point::AfterMainCommit => (identity.instance.as_str(), scope),
        };
        let heads = inspect(
            &stage,
            &root.join(format!("inspect-{index}")),
            main_instance,
            main_scope,
        )
        .await?;
        ensure!(
            heads[usage_ledger::BRANCH] != template.head(usage_ledger::BRANCH)?,
            "{point:?}: the usage branch was not adopted"
        );
        ensure!(
            (heads["main"] == template.head("main")?) == (point == Point::AfterUsageCommit),
            "{point:?}: main's adoption state is not the hook's"
        );
        stages.push(stage);
    }
    Ok(stages)
}

#[cfg(not(unix))]
async fn mid_adoption_stages(
    _root: &Path,
    _template: &Template,
    _data_dir: &Path,
    _scope: &str,
) -> Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

/// Class U: an unready template stage is preserved under `interrupted/`
/// without an engine start whatever point adoption reached: before its
/// first start, and after its identity was initialized; on Unix also after
/// the usage branch's adoption commit alone and after both commits. One open
/// preserves them all, and none of them is adopted again.
#[tokio::test]
async fn unready_template_stage_is_preserved_without_engine_start() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let scope = scope('9');
        let before_start = template_stage(&template, &data_dir, &scope)?;
        let initialized = template_stage(&template, &data_dir, &scope)?;
        start(&data_dir, &initialized, &scope, false)
            .await?
            .close()
            .await?;
        let identity = read_identity_view(&initialized)?;
        ensure!(
            identity.initialized && !initialized.join("ready.json").exists(),
            "the adopted stage is not initialized and unready"
        );
        let mut stages = vec![before_start, initialized];
        stages.extend(mid_adoption_stages(root.path(), &template, &data_dir, &scope).await?);
        let before = stages
            .iter()
            .map(|stage| Ok((ledger_key(stage)?, 0)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|(key, _)| {
                let count = starts(&key);
                (key, count)
            })
            .collect::<Vec<_>>();
        let store = open(&data_dir, &scope).await?;
        let checked = async {
            for (stage, (key, count)) in stages.iter().zip(&before) {
                ensure!(
                    starts(key) == *count,
                    "recovery started an engine on the unready template stage {}",
                    stage.display()
                );
                let kept = preserved_quiescence(&data_dir, stage).await?;
                ensure!(
                    !stage.exists()
                        && !kept.join("ready.json").exists()
                        && read_identity_view(&kept)?.template.as_deref()
                            == Some(compiled_template_key()),
                    "the unready template stage {} was not preserved as it was",
                    stage.display()
                );
            }
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    root.release(outcome)
}

/// Run the adopt job in its own task with a ready-marker pause at the
/// boundary after (or before) the marker, and return its observation, the
/// release, and the job.
async fn paused_adoption(
    data_dir: &Path,
    scope: &str,
    stage: &Path,
    after_marker: bool,
) -> Result<(
    marker_fixture::ReadyMarkerObservation,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<()>>,
)> {
    let (observation, release, pause) = marker_fixture::pause(after_marker);
    let (data_dir, scope, stage) = (data_dir.to_owned(), scope.to_owned(), stage.to_owned());
    let job = tokio::spawn(async move {
        let mut pause = Some(pause);
        adopt(&data_dir, &scope, &stage, &mut pause).await
    });
    let observed = tokio::time::timeout(
        crate::test_support::server_start_budget().saturating_add(QUERY_TIMEOUT),
        observation,
    )
    .await;
    match observed {
        Ok(Ok(observed)) => Ok((observed, release, job)),
        Ok(Err(_)) => {
            let ended = job.await?;
            bail!("the adopt job ended before its ready-marker boundary: {ended:?}")
        }
        Err(_) => {
            // Releasing lets the job finish its own cleanup before the root
            // is released.
            drop(release);
            let _ = job.await;
            bail!("the adopt job did not reach its ready-marker boundary")
        }
    }
}

/// A template stage that published `ready.json` is reused by the ordinary
/// recovery path without adoption running again: stopped after the marker,
/// the next open activates exactly that stage at its adoption head. Stopped
/// before the marker, the job preserves the stage and the next open creates
/// another store.
#[tokio::test]
async fn ready_template_stage_is_reused() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        for (digit, after_marker) in [('a', true), ('b', false)] {
            let scope = scope(digit);
            let stage = template_stage(&template, &data_dir, &scope)?;
            let (observed, release, job) =
                paused_adoption(&data_dir, &scope, &stage, after_marker).await?;
            ensure!(
                observed.after_marker == after_marker
                    && stage.join("ready.json").exists() == after_marker,
                "the pause was not at the requested marker boundary"
            );
            drop(release);
            let ended = job.await?;
            ensure!(
                ended.is_err_and(|error| format!("{error:#}").contains("observer closed before release")),
                "the cancelled adopt job did not fail at its pause"
            );
            let key = ledger_key(if after_marker {
                stage.clone()
            } else {
                preserved(&stage)?
            }
            .as_path())?;
            let adopted_starts = starts(&key);
            let store = open(&data_dir, &scope).await?;
            let checked = async {
                let active = project_directory(&data_dir, &scope)?;
                let same = files::directory(&active)?.identity().to_bytes() == observed.identity;
                if after_marker {
                    ensure!(same, "the ready template stage was not the store activated");
                    ensure!(
                        store.revision().await? == observed.initial_revision,
                        "the reused store is not at its adoption head"
                    );
                    let history: i64 = bounded(
                        "count main history",
                        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_log")
                            .fetch_one(store.pool.as_ref()),
                    )
                    .await?;
                    let adoptions: i64 = bounded(
                        "count adoption commits",
                        sqlx::query_scalar(
                            "SELECT COUNT(*) FROM dolt_log WHERE message = 'Adopt Kuru memory template'",
                        )
                        .fetch_one(store.pool.as_ref()),
                    )
                    .await?;
                    let (adopted_main, _) =
                        migrations::template_shape::compiled_commits(true)?;
                    ensure!(
                        u64::try_from(history).ok() == Some(adopted_main) && adoptions == 1,
                        "reuse changed main's history: {history} commits (expected \
                         {adopted_main}), {adoptions} adoptions"
                    );
                    // One inspection start and one active start, after the job's.
                    ensure!(
                        starts(&key) == adopted_starts + 2,
                        "the ready stage was not reused through one inspection start"
                    );
                } else {
                    ensure!(!same, "a stage stopped before its marker was activated");
                    let kept = preserved(&stage)?;
                    ensure!(
                        !kept.join("ready.json").exists()
                            && read_identity_view(&kept)?.initialized
                            && starts(&key) == adopted_starts,
                        "the stage stopped before its marker was not preserved unready"
                    );
                }
                Ok(())
            }
            .await;
            store.close().await?;
            checked?;
        }
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// The template shape on the stage engine refuses a copy whose bytes hold
/// more than the template may: a commit beyond the compiled history, a
/// branch outside the compiled set, a project-data row, a view, or more
/// tables than the check reads (which a truncated read would leave
/// unchecked). Each is a typed verdict after adoption and before
/// `ready.json`, the job preserves the unready stage, and no active
/// directory appears.
#[tokio::test]
async fn template_shape_violation_prevents_ready_marker() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let amend = format!(
            "CALL DOLT_COMMIT('-A', '--amend', '-m', 'amended', '--author', {VARIANT_AUTHOR})"
        );
        let cases = [
            (
                "extra commit",
                vec![format!(
                    "CALL DOLT_COMMIT('--allow-empty', '-m', 'extra', '--author', {VARIANT_AUTHOR})"
                )],
                "commits, not",
            ),
            (
                "extra branch",
                vec!["CALL DOLT_BRANCH('extra_branch')".to_owned()],
                "unexpected branch",
            ),
            (
                "extra row",
                vec![
                    "USE `kuru/kuru_usage_v1`".to_owned(),
                    "INSERT INTO state (`key`, value) VALUES ('extra', 'row')".to_owned(),
                    amend.clone(),
                ],
                "rows of project data",
            ),
            (
                "view",
                vec![
                    "USE `kuru/kuru_usage_v1`".to_owned(),
                    "CREATE VIEW extra_view AS SELECT 1 AS one".to_owned(),
                    amend.clone(),
                ],
                "VIEW",
            ),
            (
                "table overflow",
                std::iter::once("USE `kuru/kuru_usage_v1`".to_owned())
                    .chain(
                        (0..=256).map(|index| {
                            format!("CREATE TABLE extra_{index:03} (id INT PRIMARY KEY)")
                        }),
                    )
                    .chain(std::iter::once(amend.clone()))
                    .collect(),
                "more than 256 tables",
            ),
        ];
        for (index, (label, statements, expected)) in cases.iter().enumerate() {
            let source = variant_of(
                &template,
                &root.path().join(format!("variant-{index}")),
                statements,
            )
            .await?;
            let scope = format!("project/{}", format!("{:x}", index + 10).repeat(64));
            let stage = template_stage(&source, &data_dir, &scope)?;
            let error = adopt(&data_dir, &scope, &stage, &mut None)
                .await
                .err()
                .with_context(|| format!("{label}: the stage was marked ready"))?;
            let message = format!("{error:#}");
            ensure!(
                TemplateVerdict::find(&error).is_some()
                    && message.contains("memory store template shape")
                    && message.contains(expected),
                "{label}: expected a shape verdict containing {expected:?}: {message}"
            );
            let kept = preserved(&stage)?;
            ensure!(
                !stage.exists()
                    && kept.is_dir()
                    && !kept.join("ready.json").exists()
                    && read_identity_view(&kept)?.initialized,
                "{label}: the adopted unready stage was not preserved"
            );
            ensure!(
                !project_directory(&data_dir, &scope)?.exists(),
                "{label}: an active directory appeared"
            );
        }
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// The key comparison applies only while a stage is uninitialized: a store
/// adopted under this build's key opens writable and read-only once its
/// identity names another key, as it would under a later build, and a ready
/// template stage naming another key is reused through the ordinary
/// inspection path.
#[tokio::test]
async fn adopted_store_opens_under_a_later_key() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");
        let later = |identity: &mut crate::server::IdentityView| {
            identity.template = Some("a-later-template-key".to_owned());
        };

        let scope_a = scope('c');
        let stage = template_stage(&template, &data_dir, &scope_a)?;
        adopt(&data_dir, &scope_a, &stage, &mut None).await?;
        open(&data_dir, &scope_a).await?.close().await?;
        let active = project_directory(&data_dir, &scope_a)?;
        edit_identity(&active, later)?;
        let store = open(&data_dir, &scope_a).await?;
        let revision = store.revision().await;
        store.close().await?;
        let revision = revision?;
        let mut read_only = options(&data_dir, &scope_a)?;
        read_only.read_only = true;
        let reader = crate::test_support::spawn_gated_open(read_only).await?;
        let read = reader.revision().await;
        reader.close().await?;
        ensure!(read? == revision, "the read-only open saw another head");
        ensure!(
            read_identity_view(&active)?.template.as_deref() == Some("a-later-template-key"),
            "an open rewrote the template marker"
        );

        let scope_b = scope('d');
        let stage = template_stage(&template, &data_dir, &scope_b)?;
        let (observed, release, job) = paused_adoption(&data_dir, &scope_b, &stage, true).await?;
        drop(release);
        ensure!(job.await?.is_err(), "the cancelled adopt job succeeded");
        edit_identity(&stage, later)?;
        let store = open(&data_dir, &scope_b).await?;
        let reused = store.revision().await;
        store.close().await?;
        ensure!(
            reused? == observed.initial_revision,
            "the ready stage naming a later key was not reused"
        );
        Ok(())
    }
    .await;
    root.release(outcome)
}

/// Class U is decided before the branch that starts a writable engine on a
/// stage with an identity and no marker: the stage's key is read without a
/// second record type, and recovery preserves it with no start. The same
/// stage without the template field takes today's writable recovery start,
/// which refuses the placeholder row as another identity.
#[tokio::test]
async fn template_stage_is_classified_before_the_writable_recovery_start() -> Result<()> {
    let root = fixture_root()?;
    let outcome = async {
        let template = build_template(&root.path().join("template")).await?;
        let data_dir = root.path().join("projects");

        let marked_scope = scope('e');
        let marked = template_stage(&template, &data_dir, &marked_scope)?;
        ensure!(
            crate::server::stage_template_key(&marked)?.as_deref() == Some(compiled_template_key()),
            "the stage's template key was not read"
        );
        let key = ledger_key(&marked)?;
        let store = open(&data_dir, &marked_scope).await?;
        store.close().await?;
        ensure!(
            starts(&key) == 0 && preserved_quiescence(&data_dir, &marked).await?.is_dir(),
            "the template stage was started or not preserved"
        );

        let unmarked_scope = scope('f');
        let unmarked = template_stage(&template, &data_dir, &unmarked_scope)?;
        edit_identity(&unmarked, |identity| identity.template = None)?;
        ensure!(crate::server::stage_template_key(&unmarked)?.is_none());
        let key = ledger_key(&unmarked)?;
        let error = open(&data_dir, &unmarked_scope)
            .await
            .err()
            .context("a copied stage without its marker was recovered")?;
        ensure!(
            starts(&key) == 1 && format!("{error:#}").contains("identity mismatch"),
            "the unmarked stage did not take the writable recovery start: {error:#}"
        );
        Ok(())
    }
    .await;
    root.release(outcome)
}
