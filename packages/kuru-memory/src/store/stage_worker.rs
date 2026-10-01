//! The staging pipeline of a fresh store open, as explicit jobs.
//!
//! A fresh open builds its store in a private `<name>.staging-<uuid>`
//! directory before the stage is moved onto the active path. [`StageWorker`]
//! holds the inputs every staging job needs and runs each job on its own
//! engine start:
//!
//! - [`StageWorker::init`]: create the stage, start, bootstrap, `initialize`
//!   and the optional legacy import, then close.
//! - [`StageWorker::migrate`]: start again and hand the server to the
//!   migration worker, which owns and reaps it.
//! - [`StageWorker::validate_and_mark`]: start again, validate, and publish
//!   `ready.json` between the marker boundaries, then close.
//! - [`StageWorker::adopt_and_mark`]: the one start of a stage whose `data/`
//!   was copied from a store template and whose identity names it. The
//!   supervisor's bootstrap adopts the copy; the job then validates it,
//!   checks the template shape and publishes `ready.json` as above.
//!
//! Every job receives the startup lock and returns it only after its engine
//! has been reaped: the lock is each server's reap guard, so a cancelled
//! opener never releases it before the supervisor reaps. A job that fails
//! after its engine and main pool are open preserves the unready stage under
//! `interrupted/` itself. A failure before that (the engine start, including
//! its bootstrap and therefore adoption and any template verdict, or the main
//! pool) leaves the stage in place; the next open's recovery preserves it,
//! and for a template stage (Class U) does so without an engine start.
//! Quiescence, the move onto the active path and the active start stay with
//! the caller.
use super::*;

/// One staging engine start. Each keeps its own startup and pool error text.
#[derive(Clone, Copy)]
enum Start {
    Init,
    Migrate,
    Validate,
    Adopt,
    TemplateBuild,
}

impl Start {
    fn server_context(self) -> &'static str {
        match self {
            Self::Init => "open staged memory server",
            Self::Migrate => "reopen staged memory server for migration",
            Self::Validate => "reopen migrated staged memory server",
            Self::Adopt => "open and adopt the copied template stage",
            Self::TemplateBuild => "open the store template build server",
        }
    }

    fn pool_context(self) -> &'static str {
        match self {
            Self::Init | Self::Migrate => "open staged main pool",
            Self::Validate => "open migrated staged main pool",
            Self::Adopt => "open adopted template stage main pool",
            Self::TemplateBuild => "open the store template build main pool",
        }
    }
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
    pub(super) legacy: Option<&'a LegacyImport>,
    #[cfg(test)]
    pub(super) migration_hooks: Option<Arc<migrations::MigrationRunnerHooks>>,
    #[cfg(test)]
    pub(super) migrated_stage_pool_delay: Option<(Duration, Arc<AtomicBool>)>,
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
    ) -> Result<(Server, Arc<MySqlPool>)> {
        progress.report(MemoryOpenStage::OpeningDatabase);
        let server =
            Server::open_with_guard((self.make_options)(self.stage.to_owned(), false), startup)
                .await
                .context(start.server_context())?;
        #[cfg(test)]
        if matches!(start, Start::Validate)
            && let Some((delay, entered)) = self.migrated_stage_pool_delay.clone()
        {
            server.delay_next_pool_authentication(delay, entered);
        }
        let pool = match server.pool("main").await.context(start.pool_context()) {
            Ok(pool) => pool,
            Err(error) => return Err(close_failed_open(&server, error).await),
        };
        Ok((server, pool))
    }

    /// Create the stage and initialize it on engine start 1: bootstrap,
    /// `initialize` and the optional legacy import.
    pub(super) async fn init(
        &self,
        startup: File,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        private_dir(self.stage)?;
        let (server, pool) = self.start(startup, Start::Init, progress).await?;
        let initialized = async {
            initialize(&pool).await?;
            if let Some(legacy) = self.legacy {
                import(&pool, legacy).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match (initialized, close_migration_worker(server, pool).await) {
            (Ok(()), Ok(returned_lock)) => Ok(returned_lock),
            (Err(error), Ok(returned_lock)) => {
                let retained_lock = returned_lock;
                if let Err(preserve) = preserve_unready_stage(
                    self.stage,
                    self.parent,
                    self.lifecycle_root,
                    self.timeout,
                )
                .await
                {
                    return Err(error.context(format!(
                        "memory staging initialization preservation also failed: {preserve:#}"
                    )));
                }
                drop(retained_lock);
                Err(error)
            }
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "memory staging initialization cleanup also failed: {cleanup:#}"
            ))),
        }
    }

    /// Migrate the stage on engine start 2.
    pub(super) async fn migrate(
        &self,
        startup: File,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        // Migration itself is an accepted worker just as it is for an
        // existing project.  A cancelled stage opener cannot abandon
        // DDL or release its writer lock before the supervisor reaps.
        let (server, pool) = self.start(startup, Start::Migrate, progress).await?;
        #[cfg(test)]
        let (returned_lock, migrated) =
            run_migration_worker(server, pool, self.migration_hooks.clone()).await?;
        #[cfg(not(test))]
        let (returned_lock, migrated) = run_migration_worker(server, pool).await?;
        if let Err(error) = migrated {
            if let Err(preserve) =
                preserve_unready_stage(self.stage, self.parent, self.lifecycle_root, self.timeout)
                    .await
            {
                return Err(error.context(format!(
                    "memory staging migration preservation also failed: {preserve:#}"
                )));
            }
            return Err(error);
        }
        Ok(returned_lock)
    }

    /// Validate the migrated stage on engine start 3 and publish its
    /// `ready.json` between the marker boundaries.
    pub(super) async fn validate_and_mark(
        &self,
        startup: File,
        marker_pause: &mut Option<marker_fixture::ReadyMarkerPause>,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        self.mark(startup, Start::Validate, marker_pause, progress)
            .await
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
    /// failure. The initial revision is the adoption head of `main`.
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
        self.mark(startup, Start::Adopt, marker_pause, progress)
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
            initialize(&pool).await?;
            migrations::upgrade(&server, &pool).await?;
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
            let hostname: String = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query_scalar("SELECT @@hostname").fetch_one(pool.as_ref()),
            )
            .await
            .context("store template build host name deadline exceeded")??;
            Ok::<_, anyhow::Error>(hostname)
        }
        .await;
        match (built, close_migration_worker(server, pool).await) {
            (Ok(hostname), Ok(guard)) => Ok((guard, hostname)),
            (Err(error), Ok(guard)) => {
                drop(guard);
                Err(error)
            }
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "store template build cleanup also failed: {cleanup:#}"
            ))),
        }
    }

    async fn mark(
        &self,
        startup: File,
        start: Start,
        marker_pause: &mut Option<marker_fixture::ReadyMarkerPause>,
        progress: &mut ProgressReporter,
    ) -> Result<File> {
        let (server, pool) = self.start(startup, start, progress).await?;
        let activated = async {
            if matches!(start, Start::Adopt) {
                // The shape check classifies the retained attempts itself;
                // classifying them here as well would repeat that work.
                migrations::validate_active_unclassified(&pool).await?;
                migrations::template_shape::check(
                    &pool,
                    migrations::template_shape::Row::Adopted {
                        instance: server.instance(),
                        project_scope: self.project_scope,
                    },
                )
                .await?;
            } else {
                migrations::validate_active(&pool).await?;
            }
            let initial_revision = revision(&pool).await?;
            let activation = Activation {
                format: 1,
                project_scope: self.project_scope.to_owned(),
                initial_revision,
                migration: self.legacy.map(|legacy| legacy.receipt.clone()),
            };
            marker_fixture::reach(
                marker_pause,
                marker_fixture::Boundary::Before,
                self.stage,
                &activation,
            )
            .await?;
            write_json(&self.stage.join("ready.json"), &activation)?;
            marker_fixture::reach(
                marker_pause,
                marker_fixture::Boundary::After,
                self.stage,
                &activation,
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match (activated, close_migration_worker(server, pool).await) {
            (Ok(()), Ok(returned_lock)) => Ok(returned_lock),
            (Err(error), Ok(returned_lock)) => {
                let retained_lock = returned_lock;
                if !self.stage.join("ready.json").exists()
                    && let Err(preserve) = preserve_unready_stage(
                        self.stage,
                        self.parent,
                        self.lifecycle_root,
                        self.timeout,
                    )
                    .await
                {
                    return Err(error.context(format!(
                        "memory staging activation preservation also failed: {preserve:#}"
                    )));
                }
                drop(retained_lock);
                Err(error)
            }
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "memory staging activation cleanup also failed: {cleanup:#}"
            ))),
        }
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
    /// preserves the unready stage. On the cold path the opener owns the
    /// validation start, so cancellation drops it and the owner's reaper
    /// keeps the lock until the supervisor has exited; on the template path
    /// the creation worker owns the stage's one start and keeps the lock
    /// until it has closed and reaped that engine.
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

    /// Cancel an opener inside two stage jobs and show that a second opener
    /// acquires the startup lock only after the engine ledger shows the reap.
    ///
    /// - `migrate`: the migration worker owns the server, so the cancelled
    ///   opener's lock stays with the paused worker until it finishes, closes
    ///   and reaps.
    /// - `validate-and-mark`: the opener owns the server at the ready-marker
    ///   boundary, so cancellation drops it and the owner's reaper keeps the
    ///   lock until the supervisor has exited.
    ///
    /// The init job has no pause point; it releases the lock through the same
    /// engine close and owner drop as the other two.
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
            // The pause is in the cold staged build's migration job.
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
            next_open_preserves_the_cancelled_stage(&options).await?;
        }

        // Validate-and-mark: cancel at the boundary before `ready.json`.
        cancel_at_the_ready_marker('7', Creation::Cold).await
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
}
