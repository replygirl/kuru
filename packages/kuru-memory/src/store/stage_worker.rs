//! The staging pipeline of a fresh store open, as explicit jobs.
//!
//! A fresh open builds its store in a private `<name>.staging-<uuid>`
//! directory before the stage is moved onto the active path. [`StageWorker`]
//! holds the inputs every staging job needs, and each job makes one engine
//! start:
//!
//! - [`StageWorker::build_cold`]: the cold staged build. It creates the stage,
//!   starts its engine (whose bootstrap creates the database and identity)
//!   and hands the server to a staging worker task, which runs `initialize`,
//!   the optional legacy import at schema 1, the migration chain, validation
//!   and the revision read, publishes `ready.json` between the marker
//!   boundaries, then closes and reaps.
//! - [`StageWorker::adopt_and_mark`]: the one start of a stage whose `data/`
//!   was copied from a store template and whose identity names it. The
//!   supervisor's bootstrap adopts the copy; the job then validates it,
//!   checks the template shape and publishes `ready.json` as above.
//! - [`StageWorker::build_template`]: the one start of a store template
//!   build: the same initialization and migration chain as the cold build,
//!   then the usage branch's chain, validation and the template shape.
//!
//! Every job receives its engine's reap guard (the startup lock, or a template
//! build's exclusive key lock) and returns it only after that engine has been
//! reaped, so a cancelled opener never releases it before the supervisor
//! reaps. The cold build's worker task owns its server from the hand-over
//! after the start, in the shape of the migration worker: a cancelled opener
//! cannot abandon DDL, and the worker runs on to its ready marker or its
//! failure. A job that fails after its engine and main pool are open
//! preserves the unready stage under `interrupted/` itself. A failure before
//! that (the engine start, including its bootstrap and therefore adoption and
//! any template verdict, or the main pool) leaves the stage in place; the
//! next open's recovery preserves it, and for a template stage (Class U) does
//! so without an engine start. Quiescence, the move onto the active path and
//! the active start stay with the caller.
use super::*;

/// One staging engine start. Each keeps its own startup and pool error text.
#[derive(Clone, Copy)]
enum Start {
    Cold,
    Adopt,
    TemplateBuild,
}

impl Start {
    fn server_context(self) -> &'static str {
        match self {
            Self::Cold => "open staged memory server",
            Self::Adopt => "open and adopt the copied template stage",
            Self::TemplateBuild => "open the store template build server",
        }
    }

    fn pool_context(self) -> &'static str {
        match self {
            Self::Cold => "open staged main pool",
            Self::Adopt => "open adopted template stage main pool",
            Self::TemplateBuild => "open the store template build main pool",
        }
    }
}

/// The step of a stage's one engine session that failed, which names its
/// preservation and cleanup errors.
#[derive(Clone, Copy)]
enum Phase {
    /// `initialize` and the optional legacy import.
    Initialization,
    /// The migration chain.
    Migration,
    /// Validation, the revision read and the ready marker.
    Activation,
}

impl Phase {
    fn preservation(self) -> &'static str {
        match self {
            Self::Initialization => "memory staging initialization preservation also failed",
            Self::Migration => "memory staging migration preservation also failed",
            Self::Activation => "memory staging activation preservation also failed",
        }
    }

    fn cleanup(self) -> &'static str {
        match self {
            Self::Initialization => "memory staging initialization cleanup also failed",
            Self::Migration => "memory migration cleanup also failed",
            Self::Activation => "memory staging activation cleanup also failed",
        }
    }
}

fn in_activation(error: anyhow::Error) -> (Phase, anyhow::Error) {
    (Phase::Activation, error)
}

/// Test hooks of the cold staged build.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(super) struct StageHooks {
    /// Pauses in the stage's migration chain.
    pub(super) migration: Option<Arc<migrations::MigrationRunnerHooks>>,
    /// Delays the stage's main-pool authentication, which continues its
    /// start's deadline.
    pub(super) pool_delay: Option<(Duration, Arc<AtomicBool>)>,
    pub(super) validation: Option<ValidationProbe>,
}

/// The store directories a [`ValidationProbe`] saw validated, in order, each
/// with its native identity.
#[cfg(test)]
type Validated = Vec<(PathBuf, Option<[u8; 24]>)>;

/// Test probe of the `validate_active` calls of one writable open.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(super) struct ValidationProbe {
    /// Each store directory whose engine was about to run `validate_active`,
    /// in order, with its native identity, which follows it through a move.
    pub(super) validated: Arc<StdMutex<Validated>>,
    /// Leave an uncommitted table on the cold stage's `main` just before its
    /// validation, which then fails.
    pub(super) dirty_cold_stage: bool,
    /// Fail the active open's validation, after its engine started, with
    /// [`ACTIVE_VALIDATION_REFUSAL`].
    pub(super) refuse_active: bool,
}

/// The text of a refused active validation.
#[cfg(test)]
pub(crate) const ACTIVE_VALIDATION_REFUSAL: &str = "test active validation refusal";

#[cfg(test)]
impl ValidationProbe {
    pub(super) fn validating(&self, directory: &Path) {
        let identity = files::directory(directory)
            .ok()
            .map(|opened| opened.identity().to_bytes());
        self.validated
            .lock()
            .expect("validation probe")
            .push((directory.to_owned(), identity));
    }

    /// The active open's validation outcome under this probe.
    pub(super) fn active_outcome(&self) -> Result<()> {
        ensure!(!self.refuse_active, ACTIVE_VALIDATION_REFUSAL);
        Ok(())
    }

    async fn before_cold_stage_validation(&self, stage: &Path, pool: &MemoryPool) -> Result<()> {
        self.validating(stage);
        if self.dirty_cold_stage {
            crate::pool::within(
                QUERY_TIMEOUT,
                sqlx::query("CREATE TABLE uncommitted_fixture (id INT PRIMARY KEY)").execute(pool),
            )
            .await
            .context("cold stage dirtying deadline exceeded")??;
        }
        Ok(())
    }
}

/// `initialize` on a freshly bootstrapped store, the optional legacy import at
/// schema 1, then the migration chain on `main`: the schema work shared by the
/// cold staged build and the store template build.
async fn build_schema(
    server: &Server,
    pool: &MemoryPool,
    legacy: Option<&LegacyImport>,
    #[cfg(test)] hooks: Option<&migrations::MigrationRunnerHooks>,
) -> std::result::Result<(), (Phase, anyhow::Error)> {
    async {
        initialize(pool).await?;
        if let Some(legacy) = legacy {
            import(pool, legacy).await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await
    .map_err(|error| (Phase::Initialization, error))?;
    #[cfg(test)]
    let migrated = match hooks {
        Some(hooks) => migrations::upgrade_with_hooks(server, pool, hooks).await,
        None => migrations::upgrade(server, pool).await,
    };
    #[cfg(not(test))]
    let migrated = migrations::upgrade(server, pool).await;
    migrated.map_err(|error| (Phase::Migration, error))
}

/// The inputs shared by the staging jobs of one fresh open.
pub(super) struct StageWorker<'a, F> {
    /// Builds the engine options for a directory and access mode.
    pub(super) make_options: &'a F,
    /// The private staging directory the jobs build.
    pub(super) stage: &'a Path,
    /// The project store's parent, beneath which failed stages are preserved.
    pub(super) parent: &'a Path,
    pub(super) lifecycle_root: Option<&'a Path>,
    pub(super) timeout: Duration,
    pub(super) project_scope: &'a str,
    pub(super) legacy: Option<Arc<LegacyImport>>,
    #[cfg(test)]
    pub(super) hooks: StageHooks,
}

impl<F> StageWorker<'_, F>
where
    F: Fn(PathBuf, bool) -> ServerOptions,
{
    /// Start a writable engine on the stage with `startup` as its reap guard
    /// and open its main pool. A pool failure returns only after that engine
    /// has been reaped.
    async fn start(
        &self,
        startup: File,
        start: Start,
        progress: &mut ProgressReporter,
    ) -> Result<(Server, Arc<MemoryPool>)> {
        progress.report(MemoryOpenStage::OpeningDatabase);
        let server =
            Server::open_with_guard((self.make_options)(self.stage.to_owned(), false), startup)
                .await
                .context(start.server_context())?;
        #[cfg(test)]
        if matches!(start, Start::Cold)
            && let Some((delay, entered)) = self.hooks.pool_delay.clone()
        {
            server.delay_next_pool_authentication(delay, entered);
        }
        let pool = match server.pool("main").await.context(start.pool_context()) {
            Ok(pool) => pool,
            Err(error) => return Err(close_failed_open(&server, error).await),
        };
        Ok((server, pool))
    }

    /// The cold staged build on one engine start. Create the stage, start
    /// its engine, and hand the server to a staging worker task, which owns
    /// it until it has closed and reaped it: `initialize`, the optional legacy
    /// import, the migration chain, validation, the revision read and
    /// `ready.json` between the marker boundaries. Returns the startup lock
    /// after that reap.
    pub(super) async fn build_cold(
        &self,
        startup: File,
        marker_pause: &mut Option<marker_fixture::ReadyMarkerPause>,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        private_dir(self.stage)?;
        let (server, pool) = self.start(startup, Start::Cold, progress).await?;
        // From here the worker owns the server, as the migration worker
        // does for an existing project: a cancelled stage opener cannot
        // abandon DDL or release its writer lock before the supervisor
        // reaps, and the worker runs on to the ready marker or its failure.
        let session = self.session(Session::Cold, marker_pause.take());
        let (result, waiting) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = result.send(session.finish(server, pool).await);
        });
        waiting
            .await
            .context("memory staging worker stopped before cleanup")?
    }

    /// The one engine start of a stage copied from a store template, whose
    /// identity record names that template and is not yet initialized.
    ///
    /// The supervisor's bootstrap adopts the copy (its own instance on the
    /// usage branch and `main`, and new credentials); a failed adoption,
    /// including a [`crate::server::TemplateVerdict`], fails the start and
    /// leaves the unready stage where it is, as a failed first staging start
    /// is left today, for the next open's recovery to preserve without an
    /// engine start. After adoption this validates the store, checks the
    /// template shape with the adopted identity, and publishes `ready.json`;
    /// a failure there preserves the unready stage like any validation
    /// failure. The initial revision is the adoption head of `main`. The
    /// caller, the creation worker's task, already owns the server.
    pub(super) async fn adopt_and_mark(
        &self,
        startup: File,
        marker_pause: &mut Option<marker_fixture::ReadyMarkerPause>,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        ensure!(
            self.legacy.is_none(),
            "a legacy import never creates its store from a template"
        );
        let (server, pool) = self.start(startup, Start::Adopt, progress).await?;
        self.session(Session::Adopt, marker_pause.take())
            .finish(server, pool)
            .await
    }

    /// The one engine start of a store template build, whose identity names
    /// the template key and the placeholder instance and scope. The `guard`
    /// is the template key's exclusive lock: it is this engine's reap guard,
    /// returned only after the engine was reaped. On that one engine the
    /// bootstrap writes the placeholder row, then initialization, the
    /// migration chain, the usage branch's chain and validation, validation of
    /// `main` and the template shape with the placeholder row run. Returns the
    /// engine's `@@hostname` for the capture's byte scan. A failure leaves the
    /// build store where it is, for the next exclusive holder's sweep.
    pub(super) async fn build_template(
        &self,
        guard: File,
        progress: &mut ProgressReporter,
    ) -> Result<(File, String)> {
        ensure!(
            self.legacy.is_none(),
            "a legacy import never builds a store template"
        );
        let (server, pool) = self.start(guard, Start::TemplateBuild, progress).await?;
        let built = async {
            #[cfg(test)]
            let schema = build_schema(&server, &pool, None, None).await;
            #[cfg(not(test))]
            let schema = build_schema(&server, &pool, None).await;
            schema.map_err(|(_, error)| error)?;
            let usage = server.pool(usage_ledger::BRANCH).await?;
            migrations::upgrade_usage(&server, &usage).await?;
            migrations::validate_usage(&usage).await?;
            // The shape check classifies the retained attempts itself.
            migrations::validate_active_unclassified(&pool).await?;
            #[cfg(test)]
            super::creation_template::hooks::before_shape(&pool).await?;
            migrations::template_shape::check(&pool, migrations::template_shape::Row::Placeholder)
                .await?;
            #[cfg(test)]
            super::creation_template::hooks::after_shape(&pool).await?;
            let hostname: String = crate::pool::within(
                QUERY_TIMEOUT,
                sqlx::query_scalar("SELECT @@hostname").fetch_one(pool.as_ref()),
            )
            .await
            .context("store template build host name deadline exceeded")??;
            Ok::<_, anyhow::Error>(hostname)
        }
        .await;
        // The open returns a build failure after this close
        // (`CreateError::Build`).
        if let Err(error) = &built {
            server.mark_open_failing(error);
        }
        match (built, close_migration_worker(server, pool).await) {
            (Ok(hostname), Ok(guard)) => Ok((guard, hostname)),
            (Err(error), Ok(guard)) => {
                // The template key's exclusive lock: released explicitly, so
                // a sibling's child between fork and exec cannot keep it.
                files::release_lock(guard);
                Err(error)
            }
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "store template build cleanup also failed: {cleanup:#}"
            ))),
        }
    }

    /// The owned inputs of the session that runs on a started stage engine.
    fn session(
        &self,
        kind: Session,
        marker_pause: Option<marker_fixture::ReadyMarkerPause>,
    ) -> StageSession {
        StageSession {
            kind,
            stage: self.stage.to_owned(),
            parent: self.parent.to_owned(),
            lifecycle_root: self.lifecycle_root.map(Path::to_owned),
            timeout: self.timeout,
            project_scope: self.project_scope.to_owned(),
            legacy: self.legacy.clone(),
            marker_pause,
            #[cfg(test)]
            hooks: self.hooks.clone(),
        }
    }
}

/// What a stage's engine session does before validation.
#[derive(Clone, Copy)]
enum Session {
    /// Initialize, import and migrate the fresh stage, then validate it.
    Cold,
    /// Validate the adopted copy and check the template shape.
    Adopt,
}

/// One stage engine session after its start, owned so that it can run on a
/// worker task of its own.
struct StageSession {
    kind: Session,
    stage: PathBuf,
    parent: PathBuf,
    lifecycle_root: Option<PathBuf>,
    timeout: Duration,
    project_scope: String,
    legacy: Option<Arc<LegacyImport>>,
    marker_pause: Option<marker_fixture::ReadyMarkerPause>,
    #[cfg(test)]
    hooks: StageHooks,
}

impl StageSession {
    /// Run the session, close and reap its engine, and return the startup
    /// lock. A failure before `ready.json` preserves the unready stage while
    /// the lock is still held.
    async fn finish(mut self, server: Server, pool: Arc<MemoryPool>) -> Result<File> {
        let activated = self.activate(&server, &pool).await;
        // The open returns an activation failure after this close.
        if let Err((_, error)) = &activated {
            server.mark_open_failing(error);
        }
        match (activated, close_migration_worker(server, pool).await) {
            (Ok(()), Ok(returned_lock)) => Ok(returned_lock),
            (Err((phase, error)), Ok(returned_lock)) => {
                let retained_lock = returned_lock;
                if !self.stage.join("ready.json").exists()
                    && let Err(preserve) = preserve_unready_stage(
                        &self.stage,
                        &self.parent,
                        self.lifecycle_root.as_deref(),
                        self.timeout,
                    )
                    .await
                {
                    return Err(error.context(format!("{}: {preserve:#}", phase.preservation())));
                }
                drop(retained_lock);
                Err(error)
            }
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err((phase, error)), Err(cleanup)) => {
                Err(error.context(format!("{}: {cleanup:#}", phase.cleanup())))
            }
        }
    }

    async fn activate(
        &mut self,
        server: &Server,
        pool: &MemoryPool,
    ) -> std::result::Result<(), (Phase, anyhow::Error)> {
        match self.kind {
            Session::Cold => {
                #[cfg(test)]
                let schema = build_schema(
                    server,
                    pool,
                    self.legacy.as_deref(),
                    self.hooks.migration.as_deref(),
                )
                .await;
                #[cfg(not(test))]
                let schema = build_schema(server, pool, self.legacy.as_deref()).await;
                schema?;
                #[cfg(test)]
                if let Some(probe) = &self.hooks.validation {
                    probe
                        .before_cold_stage_validation(&self.stage, pool)
                        .await
                        .map_err(in_activation)?;
                }
                migrations::validate_active(pool)
                    .await
                    .map_err(in_activation)?;
            }
            Session::Adopt => {
                // The shape check classifies the retained attempts itself;
                // classifying them here as well would repeat that work.
                migrations::validate_active_unclassified(pool)
                    .await
                    .map_err(in_activation)?;
                migrations::template_shape::check(
                    pool,
                    migrations::template_shape::Row::Adopted {
                        instance: server.instance(),
                        project_scope: &self.project_scope,
                    },
                )
                .await
                .map_err(in_activation)?;
            }
        }
        self.mark(pool).await.map_err(in_activation)
    }

    /// Read the initial revision and publish `ready.json` between the marker
    /// boundaries.
    async fn mark(&mut self, pool: &MemoryPool) -> Result<()> {
        let initial_revision = revision(pool).await?;
        let activation = Activation {
            format: 1,
            history_scope: None,
            restore: None,
            project_scope: self.project_scope.clone(),
            initial_revision,
            migration: self.legacy.as_ref().map(|legacy| legacy.receipt.clone()),
        };
        marker_fixture::reach(
            &mut self.marker_pause,
            marker_fixture::Boundary::Before,
            &self.stage,
            &activation,
        )
        .await?;
        write_json(&self.stage.join("ready.json"), &activation)?;
        marker_fixture::reach(
            &mut self.marker_pause,
            marker_fixture::Boundary::After,
            &self.stage,
            &activation,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::engine_ledger;

    /// Reaching a pause, or the reap after it, may include a whole startup
    /// and one bounded query.
    fn observation_deadline(options: &OpenOptions) -> Duration {
        Duration::from_secs(options.config.startup_timeout_secs).saturating_add(QUERY_TIMEOUT)
    }

    /// Supervisors this process started beneath `root` and has not reaped.
    fn live_under(root: &Path) -> Vec<String> {
        engine_ledger::with(|ledger| ledger.live_under(root))
    }

    /// One non-waiting attempt on the project's startup lock, as a second
    /// opener's `acquire_lock` makes it. On success it also returns the live
    /// owners the ledger showed while this attempt held the lock. The attempt
    /// holds the lock gate, so no sibling test's child inherits the
    /// description; see `crate::spawn_gate`.
    async fn try_startup_lock(
        options: &OpenOptions,
        root: &Path,
    ) -> Result<Option<(File, Vec<String>)>> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let parent = directory.parent().context("project store has no parent")?;
        let locks = files::open_directory(
            &parent.join("locks"),
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )?;
        let name = directory.file_name().context("project store has no name")?;
        let _gate = crate::spawn_gate::locking_async().await;
        let file = locks.lock_file(name)?;
        locks.verify(name, &file)?;
        match file.try_lock() {
            Ok(()) => {
                let live = live_under(root);
                Ok(Some((file, live)))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(anyhow::anyhow!(error)).context("probe the project startup lock"),
        }
    }

    /// Poll the startup lock as a second opener would until it is acquired,
    /// release it, and return the live owners the ledger showed at the
    /// moment of acquisition.
    async fn second_opener_acquires(
        options: &OpenOptions,
        root: &Path,
        deadline: Duration,
    ) -> Result<Vec<String>> {
        let started = Instant::now();
        loop {
            if let Some((file, live)) = try_startup_lock(options, root).await? {
                let _gate = crate::spawn_gate::locking_async().await;
                drop(file);
                return Ok(live);
            }
            ensure!(
                started.elapsed() < deadline,
                "the startup lock was still held {deadline:?} after the stage opener was \
                 cancelled; live owners: {:?}",
                live_under(root)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait, bounded, until the ledger shows every supervisor beneath `root`
    /// reaped. A failed check waits here before it returns, so the fixture
    /// root's teardown does not replace the failure with a live-engine panic.
    async fn await_reaped(root: &Path, deadline: Duration) -> Result<()> {
        let started = Instant::now();
        loop {
            let live = live_under(root);
            if live.is_empty() {
                return Ok(());
            }
            ensure!(
                started.elapsed() < deadline,
                "stage engines were still live {deadline:?} after the cancelled opener: {live:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Entries preserved under the project's `interrupted/` directory.
    fn interrupted(options: &OpenOptions) -> Result<Vec<PathBuf>> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let interrupted = directory
            .parent()
            .context("project store has no parent")?
            .join("interrupted");
        let mut entries = fs::read_dir(interrupted)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        Ok(entries)
    }

    /// The project's `.staging-` directories.
    fn stages(options: &OpenOptions) -> Result<Vec<PathBuf>> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let name = format!(
            "{}.staging-",
            directory
                .file_name()
                .context("project store has no name")?
                .to_string_lossy()
        );
        let mut stages = fs::read_dir(directory.parent().context("project store has no parent")?)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|stage| stage.to_string_lossy().starts_with(&name))
            })
            .collect::<Vec<_>>();
        stages.sort();
        Ok(stages)
    }

    /// After a cancelled stage build whose worker ran on to the ready marker,
    /// the one stage left is ready and an ordinary opener reuses it: the
    /// active store is that stage directory, and nothing is preserved.
    async fn next_open_reuses_the_ready_stage(options: &OpenOptions) -> Result<()> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let stages = stages(options)?;
        let [stage] = stages.as_slice() else {
            bail!("the cancelled stage build did not leave one stage: {stages:?}");
        };
        ensure!(
            stage.join("ready.json").is_file(),
            "the worker of the cancelled stage build did not mark its stage ready"
        );
        let identity = files::directory(stage)?.identity().to_bytes();
        let store = tokio::time::timeout(
            crate::test_support::fresh_open_budget(),
            crate::test_support::spawn_gated_open(options.clone()),
        )
        .await
        .context("the open after a cancelled stage build did not finish")??;
        store.close().await?;
        ensure!(
            files::directory(&directory)?.identity().to_bytes() == identity
                && !directory.with_file_name("interrupted").exists(),
            "the next open did not reuse the ready stage"
        );
        Ok(())
    }

    /// After a cancelled stage build, an ordinary opener preserves the
    /// unready stage and completes a fresh open.
    async fn next_open_preserves_the_cancelled_stage(options: &OpenOptions) -> Result<()> {
        let store = tokio::time::timeout(
            observation_deadline(options),
            crate::test_support::spawn_gated_open(options.clone()),
        )
        .await
        .context("the open after a cancelled stage build did not finish")??;
        store.close().await?;
        let preserved = interrupted(options)?;
        ensure!(
            preserved.len() == 1 && !preserved[0].join("ready.json").exists(),
            "the cancelled unready stage was not preserved once: {preserved:?}"
        );
        Ok(())
    }

    /// Cancel an opener at the boundary before its stage's `ready.json` and
    /// show that a second opener acquires the startup lock only after the
    /// engine ledger shows the stage engine reaped, and that the next open
    /// preserves the unready stage. On either path a worker owns the stage's
    /// one start (the cold staging worker from the hand-over after the start,
    /// the creation worker on the template path), so the cancelled opener's
    /// lock stays with that worker until it has closed and reaped the engine;
    /// closing the observer fails the worker's marker step, so it preserves
    /// the unready stage first.
    async fn cancel_at_the_ready_marker(digit: char, creation: Creation) -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let canonical = fs::canonicalize(root.path())?;
        let mut options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", digit.to_string().repeat(64)),
        )
        .await?;
        options.creation = creation;
        let deadline = observation_deadline(&options);
        let (observation, release, opening) = marker_fixture::prepare(options.clone(), false);
        let opening = tokio::spawn(async move {
            let _gate = crate::spawn_gate::spawning().await;
            opening.await
        });
        let observed = match tokio::time::timeout(deadline, observation).await {
            Ok(Ok(observed)) => observed,
            Ok(Err(_)) => {
                let ended = match opening.await {
                    Ok(Ok(store)) => {
                        store.close().await?;
                        "opened".to_owned()
                    }
                    Ok(Err(error)) => format!("{error:#}"),
                    Err(error) => error.to_string(),
                };
                bail!("the stage opener ended before the ready-marker boundary: {ended}");
            }
            Err(_) => {
                opening.abort();
                let _ = opening.await;
                bail!("the stage opener did not reach the ready-marker boundary in {deadline:?}");
            }
        };
        ensure!(
            !observed.after_marker && !observed.stage.join("ready.json").exists(),
            "the ready-marker pause was not before the marker"
        );
        opening.abort();
        ensure!(
            opening.await.is_err_and(|error| error.is_cancelled()),
            "the stage opener finished instead of being cancelled at the ready marker"
        );
        // Closing the observer only now keeps the cancellation, not an
        // observer error, as the reason the opener stopped.
        drop(release);
        let acquired = second_opener_acquires(&options, &canonical, deadline).await;
        let reaped = await_reaped(&canonical, deadline).await;
        let live = acquired?;
        reaped?;
        ensure!(
            live.is_empty(),
            "a second opener acquired the startup lock before the cancelled stage engine was \
             reaped: {live:?}"
        );
        next_open_preserves_the_cancelled_stage(&options).await?;
        Ok(())
    }

    /// Cancel an opener at two points of the cold stage's one engine session
    /// and show that a second opener acquires the startup lock only after the
    /// engine ledger shows the reap.
    ///
    /// - Accepted DDL in the migration chain: the staging worker owns the
    ///   server, so the cancelled opener's lock stays with the paused worker,
    ///   which then runs on to validation and the ready marker before it
    ///   closes and reaps; the next open reuses that ready stage.
    /// - The ready-marker boundary with its observer closed: the worker
    ///   preserves the unready stage, then closes and reaps.
    ///
    /// Initialization has no pause point; it runs on the same session and
    /// releases the lock through the same close.
    #[tokio::test]
    async fn cancelled_open_during_stage_build_keeps_startup_lock_until_reap() -> Result<()> {
        // Migrate: cancel while the worker holds accepted DDL.
        {
            let root = crate::test_support::tempdir()?;
            let canonical = fs::canonicalize(root.path())?;
            let mut options = crate::test_support::warmed_open_options(
                root.path().to_owned(),
                format!("project/{}", "6".repeat(64)),
            )
            .await?;
            let deadline = observation_deadline(&options);
            let (hooks, control) =
                migrations::MigrationRunnerHooks::paused(migrations::MigrationBoundary::AfterDdl);
            // The pause is in the cold staged build's migration chain.
            options.creation = Creation::Cold;
            options.migration_hooks = Some(Arc::new(hooks));
            let opening = tokio::spawn(crate::test_support::spawn_gated_open(options.clone()));
            tokio::time::timeout(deadline, control.reached())
                .await
                .context("the stage migration did not reach accepted DDL")??;
            opening.abort();
            ensure!(
                opening.await.is_err_and(|error| error.is_cancelled()),
                "the stage opener finished instead of being cancelled at accepted DDL"
            );
            let held = try_startup_lock(&options, &canonical).await?;
            let live = live_under(&canonical);
            let paused = match held {
                Some((file, _)) => {
                    let _gate = crate::spawn_gate::locking_async().await;
                    drop(file);
                    Err(anyhow::anyhow!(
                        "the cancelled stage opener released the startup lock while its \
                         migration worker was paused; live owners: {live:?}"
                    ))
                }
                None if live.is_empty() => Err(anyhow::anyhow!(
                    "the paused migration worker's engine is missing from the ledger"
                )),
                None => Ok(()),
            };
            control.resume();
            let acquired = second_opener_acquires(&options, &canonical, deadline).await;
            let reaped = await_reaped(&canonical, deadline).await;
            paused?;
            let live = acquired?;
            reaped?;
            ensure!(
                live.is_empty(),
                "a second opener acquired the startup lock before the migration worker's engine \
                 was reaped: {live:?}"
            );
            options.migration_hooks = None;
            next_open_reuses_the_ready_stage(&options).await?;
        }

        // The ready marker: cancel at the boundary before `ready.json`.
        cancel_at_the_ready_marker('7', Creation::Cold).await
    }

    /// Supervisors this process started for stores first started beneath
    /// `root`, the key following each stage through its move onto the
    /// active path.
    fn starts_under(root: &Path) -> Result<u64> {
        let canonical = fs::canonicalize(root)?;
        Ok(engine_ledger::with(|ledger| {
            ledger.starts_under(&canonical)
        }))
    }

    /// A legacy SQLite store with one message for `scope` in `data`, which a
    /// writable open imports.
    fn legacy_source(data: &Path, scope: &str) -> Result<()> {
        files::private_dir(data)?;
        let source = rusqlite::Connection::open(data.join("memory.sqlite3"))?;
        source.execute_batch(
            "PRAGMA application_id=1263882837;
             PRAGMA user_version=1;
             CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
             CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )?;
        source.execute(
            "INSERT INTO messages (namespace, role, content) VALUES (?1, 'user', 'legacy')",
            [format!("{scope}/transcript")],
        )?;
        Ok(())
    }

    /// Open `options` paused just after its stage's `ready.json`, check that
    /// the stage was built on one engine start that is still serving there
    /// (so no engine was closed before it), and finish the open.
    async fn open_observing_one_stage_engine(options: &OpenOptions) -> Result<MemoryStore> {
        let deadline = observation_deadline(options);
        let (observation, release, opening) = marker_fixture::prepare(options.clone(), true);
        let mut opening = tokio::spawn(async move {
            let _gate = crate::spawn_gate::spawning().await;
            opening.await
        });
        let observed = tokio::time::timeout(deadline, observation).await;
        let at_marker = (
            starts_under(&options.data_dir),
            engine_ledger::with(|ledger| {
                fs::canonicalize(&options.data_dir).map(|data| ledger.live_under(&data))
            }),
        );
        let checked = match observed {
            Ok(Ok(observed)) => (|| {
                let (starts, live) = at_marker;
                let (starts, live) = (starts?, live?);
                ensure!(
                    observed.after_marker && observed.stage.join("ready.json").is_file(),
                    "the pause was not after the stage's ready marker"
                );
                ensure!(
                    starts == 1 && live.len() == 1,
                    "the stage was marked ready after {starts} engine starts, with live \
                     engines {live:?}"
                );
                Ok(())
            })(),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "the open ended before its stage's ready marker"
            )),
            Err(_) => Err(anyhow::anyhow!(
                "the open did not reach its stage's ready marker in {deadline:?}"
            )),
        };
        // Released whatever the check found, so the open finishes and its
        // engines are reaped before the fixture root is.
        let _ = release.send(());
        let opened =
            tokio::time::timeout(crate::test_support::fresh_open_budget(), &mut opening).await;
        let store = match opened {
            Ok(Ok(Ok(store))) => store,
            Ok(Ok(Err(error))) => return Err(error.context("the observed open failed")),
            Ok(Err(error)) => bail!("the observed open stopped: {error}"),
            Err(_) => {
                opening.abort();
                let _ = opening.await;
                bail!("the observed open did not finish after its ready marker");
            }
        };
        if let Err(error) = checked {
            store.close().await?;
            return Err(error);
        }
        Ok(store)
    }

    /// The cold staged build makes one engine start in its stage, closed only
    /// after `ready.json`, and the active start: two in all, for a
    /// `Creation::Cold` store and for a legacy import alike. Reopening the
    /// existing project then makes one.
    #[tokio::test]
    async fn cold_stage_initializes_migrates_and_validates_on_one_engine() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        for (name, digit, legacy) in [("cold", 'a', false), ("legacy", 'b', true)] {
            let data = root.path().join(name);
            let scope = format!("project/{}", digit.to_string().repeat(64));
            if legacy {
                legacy_source(&data, &scope)?;
            }
            let mut options =
                crate::test_support::warmed_open_options(data.clone(), scope.clone()).await?;
            if !legacy {
                // A legacy import is cold through the ordinary selector.
                options.creation = Creation::Cold;
            }
            let store = open_observing_one_stage_engine(&options).await?;
            let checked = async {
                ensure!(
                    migrations::version(&store.pool).await? == migrations::CURRENT_VERSION,
                    "the {name} store is not at the current schema"
                );
                let activation = read_activation(&project_directory(&data, &scope)?, &scope)?;
                ensure!(
                    activation.migration.is_some() == legacy,
                    "the {name} store's activation import receipt is wrong"
                );
                Ok(())
            }
            .await;
            store.close().await?;
            checked?;
            let starts = starts_under(&data)?;
            ensure!(
                starts == 2,
                "the {name} store's creation made {starts} engine starts"
            );
            let store = crate::test_support::spawn_gated_open(options).await?;
            store.close().await?;
            let starts = starts_under(&data)?;
            ensure!(
                starts == 3,
                "reopening the existing {name} project made {} engine starts",
                starts - 2
            );
        }
        Ok(())
    }

    /// A validation failure on the cold stage's one engine fails the open
    /// with that error, after the staging worker reaped the engine and
    /// preserved the unready stage under `interrupted/`, with no second
    /// engine start and nothing at the project's active path.
    #[tokio::test]
    async fn cold_stage_validation_failure_preserves_unready_stage() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let canonical = fs::canonicalize(root.path())?;
        let mut options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "d".repeat(64)),
        )
        .await?;
        options.creation = Creation::Cold;
        let probe = ValidationProbe {
            dirty_cold_stage: true,
            ..ValidationProbe::default()
        };
        options.validation_probe = Some(probe.clone());
        let opened = tokio::time::timeout(
            crate::test_support::fresh_open_budget_of(crate::test_support::FreshOpen::Cold),
            crate::test_support::spawn_gated_open(options.clone()),
        )
        .await
        .context("the cold open with a dirty stage did not finish")?;
        let error = match opened {
            Ok(store) => {
                store.close().await?;
                bail!("a cold stage that failed its validation opened");
            }
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            error.contains("uncommitted changes") && !error.contains("also failed"),
            "the open did not fail with the stage's validation error alone: {error}"
        );
        let live = live_under(&canonical);
        ensure!(
            live.is_empty(),
            "the failed open returned before its stage engine was reaped: {live:?}"
        );
        let starts = starts_under(root.path())?;
        let validated = probe.validated.lock().expect("validation probe").clone();
        ensure!(
            starts == 1 && validated.len() == 1,
            "the failed cold build made {starts} engine starts and validated {validated:?}"
        );
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let preserved = interrupted(&options)?;
        ensure!(
            !directory.exists()
                && stages(&options)?.is_empty()
                && matches!(
                    preserved.as_slice(),
                    [stage] if stage.file_name() == validated[0].0.file_name()
                        && !stage.join("ready.json").exists()
                ),
            "the unready stage was not preserved once, or a store appeared at the active \
             path: {preserved:?}"
        );
        Ok(())
    }

    /// An opener cancelled after the migration chain, at the boundary before
    /// the stage's `ready.json`, leaves the stage's engine with the staging
    /// worker, which owns it from the hand-over after the start: the startup
    /// lock stays held while the worker is paused, and once released the
    /// worker publishes `ready.json`, closes and reaps, and only then
    /// returns the lock. The next open reuses the ready stage.
    #[tokio::test]
    async fn cancelled_cold_open_after_migration_reaps_before_lock_release() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let canonical = fs::canonicalize(root.path())?;
        let mut options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "c".repeat(64)),
        )
        .await?;
        options.creation = Creation::Cold;
        let deadline = observation_deadline(&options);
        let (observation, release, opening) = marker_fixture::prepare(options.clone(), false);
        let opening = tokio::spawn(async move {
            let _gate = crate::spawn_gate::spawning().await;
            opening.await
        });
        let observed = match tokio::time::timeout(deadline, observation).await {
            Ok(Ok(observed)) => observed,
            Ok(Err(_)) => {
                let ended = match opening.await {
                    Ok(Ok(store)) => {
                        store.close().await?;
                        "opened".to_owned()
                    }
                    Ok(Err(error)) => format!("{error:#}"),
                    Err(error) => error.to_string(),
                };
                bail!("the cold opener ended before the ready-marker boundary: {ended}");
            }
            Err(_) => {
                opening.abort();
                let _ = opening.await;
                bail!("the cold opener did not reach the ready-marker boundary in {deadline:?}");
            }
        };
        opening.abort();
        let cancelled = opening.await.is_err_and(|error| error.is_cancelled());
        // Probe before releasing the pause, so the worker is still paused.
        let held = try_startup_lock(&options, &canonical).await;
        let live = live_under(&canonical);
        let paused = match held {
            Ok(Some((file, _))) => {
                let _gate = crate::spawn_gate::locking_async().await;
                drop(file);
                Err(anyhow::anyhow!(
                    "the cancelled cold opener released the startup lock while its staging \
                     worker was paused; live owners: {live:?}"
                ))
            }
            Ok(None) if live.len() != 1 => Err(anyhow::anyhow!(
                "the paused staging worker's engine is not the one live engine: {live:?}"
            )),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        // Released, not closed: the worker runs on to the ready marker.
        let _ = release.send(());
        let acquired = second_opener_acquires(&options, &canonical, deadline).await;
        let reaped = await_reaped(&canonical, deadline).await;
        ensure!(
            cancelled,
            "the cold opener finished instead of being cancelled at the ready marker"
        );
        ensure!(
            !observed.after_marker,
            "the ready-marker pause was not before the marker"
        );
        paused?;
        let live = acquired?;
        reaped?;
        ensure!(
            live.is_empty(),
            "a second opener acquired the startup lock before the staging worker's engine was \
             reaped: {live:?}"
        );
        ensure!(
            files::directory(&observed.stage)?.identity().to_bytes() == observed.identity,
            "the observed stage was moved before the next open"
        );
        next_open_reuses_the_ready_stage(&options).await
    }

    /// `validate_active` runs on the cold stage's engine and again on the
    /// active engine after the stage was stopped, moved onto the active path
    /// and reloaded from disk: the same directory, two engine starts.
    #[tokio::test]
    async fn active_open_validates_after_reload() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let mut options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "e".repeat(64)),
        )
        .await?;
        options.creation = Creation::Cold;
        let probe = ValidationProbe::default();
        options.validation_probe = Some(probe.clone());
        let store = crate::test_support::spawn_gated_open(options.clone()).await?;
        store.close().await?;
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        let validated = probe.validated.lock().expect("validation probe").clone();
        let starts = starts_under(root.path())?;
        ensure!(
            matches!(
                validated.as_slice(),
                [(stage, Some(staged)), (active, Some(reloaded))]
                    if stage.file_name().is_some_and(|name| {
                        name.to_string_lossy().contains(".staging-")
                    }) && *active == directory
                        && staged == reloaded
            ) && starts == 2,
            "validation did not run on the stage and again on the reloaded active store \
             ({starts} engine starts): {validated:?}"
        );
        Ok(())
    }

    /// The template path's creation worker owns the startup lock, the copy
    /// and the stage's one engine start: an opener cancelled at the boundary
    /// before the stage's `ready.json` leaves the lock with the worker until
    /// that engine was reaped, and the next open preserves the stage.
    #[tokio::test]
    async fn cancelled_open_during_template_copy_keeps_startup_lock_until_reap() -> Result<()> {
        cancel_at_the_ready_marker('8', Creation::Default).await
    }

    /// The first project for a key is cancelled while its creation worker is
    /// inside the template build. The worker owns the startup lock and runs
    /// on: it publishes the template, copies the project from it, adopts and
    /// marks the stage ready, and returns the lock only after its engines
    /// were reaped. The next open reuses that ready stage.
    #[tokio::test]
    async fn cancelled_open_during_template_build_finishes_and_leaves_a_ready_stage() -> Result<()>
    {
        use crate::store::creation_template::hooks::{HOOKS, Hooks, Pause};
        // The private templates root, one level down, holds the template's
        // captured database repository.
        let root = crate::test_support::tempdir()?
            .with_depth_budget(1 + crate::test_support::TEMPLATE_DEPTH);
        let canonical = fs::canonicalize(root.path())?;
        let templates = root.path().join("templates");
        let mut options = crate::test_support::warmed_open_options(
            root.path().join("data"),
            format!("project/{}", "9".repeat(64)),
        )
        .await?;
        options.template_root = Some(templates.clone());
        let bound =
            crate::test_support::fresh_open_budget_of(crate::test_support::FreshOpen::FirstProject);
        let pause = Arc::new(Pause::default());
        let hooks = Hooks {
            pause: Some(pause.clone()),
            ..Hooks::default()
        };
        let mut opening = tokio::spawn(HOOKS.scope(
            hooks,
            crate::test_support::spawn_gated_open(options.clone()),
        ));
        let waiting = &mut opening;
        let paused = tokio::time::timeout(bound, async {
            tokio::select! {
                () = pause.reached.notified() => None,
                finished = waiting => Some(finished),
            }
        })
        .await;
        let missed = match paused {
            Ok(None) => None,
            Ok(Some(finished)) => Some(match finished {
                Ok(Ok(store)) => format!(
                    "the open finished without the build (close: {:?})",
                    store.close().await
                ),
                Ok(Err(error)) => format!("the open failed: {error:#}"),
                Err(error) => format!("the open stopped: {error}"),
            }),
            Err(_) => {
                opening.abort();
                let _ = (&mut opening).await;
                Some(format!("the open did not reach it in {bound:?}"))
            }
        };
        if let Some(missed) = missed {
            let reaped = await_reaped(&canonical, bound).await;
            return root.release(reaped.and_then(|()| {
                bail!("the first project did not reach its template build's pause: {missed}")
            }));
        }
        opening.abort();
        let cancelled = opening.await.is_err_and(|error| error.is_cancelled());
        // Probe the lock before resuming, so the worker is still paused.
        let held = try_startup_lock(&options, &canonical)
            .await
            .map(|acquired| acquired.is_none());
        pause.resume.notify_one();
        let acquired = second_opener_acquires(&options, &canonical, bound).await;
        // Always awaited, so a failed check reaches the fixture's release
        // with its own error rather than as a live engine at teardown.
        let reaped = await_reaped(&canonical, bound).await;
        let outcome = async {
            ensure!(
                cancelled,
                "the opener finished instead of being cancelled in the build"
            );
            ensure!(
                held?,
                "the startup lock was free while the creation worker was inside the build"
            );
            let live = acquired?;
            reaped?;
            ensure!(
                live.is_empty(),
                "a second opener acquired the startup lock before the creation worker's \
                 engines were reaped: {live:?}"
            );
            let key = super::super::creation_template::compiled_key();
            ensure!(
                templates.join(key).join("manifest.json").is_file(),
                "the cancelled opener's worker did not publish the template"
            );
            let directory = project_directory(&options.data_dir, &options.project_scope)?;
            let name = format!(
                "{}.staging-",
                directory
                    .file_name()
                    .context("project store has no name")?
                    .to_string_lossy()
            );
            let stages = fs::read_dir(directory.parent().context("project store has no parent")?)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|stage| stage.to_string_lossy().starts_with(&name))
                })
                .collect::<Vec<_>>();
            ensure!(
                matches!(stages.as_slice(), [stage] if stage.join("ready.json").is_file()),
                "the worker did not leave one ready stage: {stages:?}"
            );
            let store = tokio::time::timeout(
                bound,
                crate::test_support::spawn_gated_open(options.clone()),
            )
            .await
            .context("the open after a cancelled template build did not finish")??;
            store.close().await?;
            let template = crate::server::read_identity_view(&directory)?.template;
            ensure!(
                template.as_deref() == Some(key)
                    && !directory.with_file_name("interrupted").exists(),
                "the next open did not reuse the ready template stage (store template \
                 {template:?})"
            );
            Ok(())
        }
        .await;
        root.release(outcome)
    }

    /// In-process cold-path open timing (not an assertion): times a plain
    /// `Creation::Cold` open and a one-message legacy-import open through
    /// `test_support::spawn_gated_open`, the harness behind the proposal's
    /// benchmark rows. Run explicitly, alone (one engine start competes for
    /// the host with another):
    ///
    /// ```text
    /// KURU_TEST_COLD_OPEN_MEASURE_DIR=<dir> KURU_TEST_COLD_OPEN_MEASURE_ITERATIONS=<n> \
    ///   cargo test -p kuru-memory --lib --all-features --locked \
    ///   stage_worker::tests::measure_cold_open_latency -- --ignored --test-threads=1
    /// ```
    ///
    /// Each iteration opens a fresh store in its own directory, times the
    /// open to a ready `MemoryStore`, then closes it. Rows are written as
    /// CSV beneath the measurement directory; nothing is printed, so the
    /// test is safe beside PTY fixtures. This reproduces the shape of the
    /// base/head comparison in `proposal.md`'s Benchmarks table (run once
    /// per checkout and diff the summaries) without committing two
    /// executables; it is not itself an interleaved base-vs-head run.
    #[tokio::test]
    #[ignore = "measurement: run explicitly with --ignored (see doc comment)"]
    async fn measure_cold_open_latency() -> Result<()> {
        use std::{fmt::Write as _, io::Write as _};

        let directory = std::env::var_os("KURU_TEST_COLD_OPEN_MEASURE_DIR")
            .filter(|value| !value.is_empty())
            .map_or_else(
                || std::env::temp_dir().join("kuru-cold-open-measurements"),
                PathBuf::from,
            );
        fs::create_dir_all(&directory)?;
        let iterations: usize = std::env::var("KURU_TEST_COLD_OPEN_MEASURE_ITERATIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10);
        let stamp = crate::test_support::lifecycle_trace::nanos();
        let mut csv = fs::File::create(directory.join(format!("cold-open-{stamp}.csv")))?;
        writeln!(csv, "kind,iteration,elapsed_ms")?;
        let mut summary = String::new();
        let root = crate::test_support::tempdir()?;
        for (name, legacy) in [("cold", false), ("legacy", true)] {
            let mut millis = Vec::with_capacity(iterations);
            for index in 0..iterations {
                let data = root.path().join(format!("{name}-{index}"));
                let scope = format!("project/{index:064x}");
                if legacy {
                    legacy_source(&data, &scope)?;
                }
                let mut options =
                    crate::test_support::warmed_open_options(data.clone(), scope).await?;
                if !legacy {
                    options.creation = Creation::Cold;
                }
                let started = Instant::now();
                let store = crate::test_support::spawn_gated_open(options).await?;
                let elapsed = started.elapsed();
                store.close().await?;
                writeln!(csv, "{name},{index},{}", elapsed.as_millis())?;
                millis.push(elapsed.as_millis());
            }
            millis.sort_unstable();
            let p50 = millis.get(millis.len() / 2).copied().unwrap_or_default();
            let _ = writeln!(
                summary,
                "kind={name} iterations={iterations} min={} p50={p50} max={}",
                millis.first().copied().unwrap_or_default(),
                millis.last().copied().unwrap_or_default(),
            );
        }
        fs::write(
            directory.join(format!("cold-open-{stamp}-summary.txt")),
            summary,
        )?;
        Ok(())
    }
}
