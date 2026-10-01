//! A fresh open's choice of creation path, and the worker that creates a new
//! store from the per-machine store template.
//!
//! [`select`] runs once, after recovery found no stage to reuse. A legacy
//! import, a configured engine binary and the test-support `Creation::Cold`
//! keep the open on the cold staged build (`MemoryStore::create_cold`, four
//! engine starts). Every other new store is created from the template.
//!
//! [`run`] hands the startup lock to a creation worker task, the ownership
//! shape of the migration worker: the opener's frame holds no lock while the
//! work is in flight, and a cancelled opener leaves the worker to finish. The
//! worker copies the template into a new `.staging-<uuid>` stage under the
//! key's shared lock, which the blocking copy holds until it returns
//! ([`creation_template::create_in`]). When no template is published yet it
//! first builds one under the key's exclusive lock, which is that build
//! engine's reap guard, and copies from it. It then writes the stage's
//! identity record last and runs the stage's one engine start
//! ([`stage_worker::StageWorker::adopt_and_mark`]): adoption, validation, the
//! template shape and `ready.json`. The startup lock comes back only after
//! that engine was reaped. Quiescence, the move onto the active path and the
//! active start stay with the opener, unchanged.
//!
//! Engine starts: two with a published template (the stage and the active
//! store), three when this open builds the template.
//!
//! Where the template cannot be used the worker hands the startup lock back
//! for the cold path, never failing the open: a busy key lock, a lock-file
//! error, or a verdict or I/O error before or during the copy. A copy that
//! wrote anything is preserved first under `interrupted/` without an engine
//! start (recovery's Class R); an empty stage is removed. Only a failed
//! preservation fails the open there.
//!
//! Any failure of this open's template build (its engine, its own
//! assertions, its capture or its publication) and any failure of the
//! stage's own engine start return their error with no retry, on either
//! path: the chain is never paid twice in one open. A verdict on the stage's
//! engine (adoption or the template shape) also quarantines the template the
//! copy came from, identity-bound and best-effort; every other failure
//! leaves every template untouched. A failed stage start leaves the unready
//! stage in place for the next open's recovery to preserve without an engine
//! start (Class U); a failure after the stage's engine served preserves it
//! at once.
use super::*;
use creation_template::{CreateError, Created};

/// The template root a new store is created from, or `None` for the cold
/// staged build.
pub(super) fn select(options: &OpenOptions, legacy: bool) -> Option<PathBuf> {
    // The import runs at schema 1, before the chain.
    if legacy {
        return None;
    }
    // The template key binds the pinned managed engine's digest, which a
    // configured executable does not have.
    if options.config.dolt_binary.is_some() {
        return None;
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        if options.creation == Creation::Cold {
            return None;
        }
        if let Some(root) = &options.template_root {
            return Some(root.clone());
        }
    }
    // The engine cache provisioning just used and canonicalized.
    let cache = options
        .config
        .cache_dir
        .clone()
        .unwrap_or_else(|| options.data_dir.join("tools/dolt"));
    match fs::canonicalize(&cache) {
        Ok(cache) => Some(creation_template::root_in(&cache)),
        Err(error) => {
            tracing::warn!(
                cache = %cache.display(),
                error = %error,
                "store template cache unavailable; creating without a template"
            );
            None
        }
    }
}

/// Everything the creation worker owns.
pub(super) struct TemplateCreation {
    /// The templates root: `<cache>/<engine version>/templates`.
    pub(super) root: PathBuf,
    pub(super) engine: creation_template::Engine,
    /// The stage engine's options; each start replaces only its directory
    /// and access mode.
    pub(super) base: ServerOptions,
    /// The new `<name>.staging-<uuid>` directory.
    pub(super) stage: PathBuf,
    /// The project store's parent, beneath which failed stages are preserved.
    pub(super) parent: PathBuf,
    pub(super) project_scope: String,
    pub(super) marker_pause: Option<marker_fixture::ReadyMarkerPause>,
}

/// How the creation worker ended when it returned the startup lock.
pub(super) enum Outcome {
    /// The stage holds an adopted, validated store and its `ready.json`.
    Ready,
    /// No usable template: create the store cold in a new stage. Carries
    /// back the ready-marker pause the worker did not reach.
    Cold(Option<marker_fixture::ReadyMarkerPause>),
}

/// Run `job` on its own task, which owns `startup` (and every lock and engine
/// it takes) until its engines are reaped, and return its outcome with the
/// startup lock. `Err` means every engine the worker started was reaped.
pub(super) async fn run(job: TemplateCreation, startup: File) -> Result<(File, Outcome)> {
    let (result, waiting) = tokio::sync::oneshot::channel();
    // Task-local hooks do not cross `tokio::spawn`.
    #[cfg(test)]
    let scoped = creation_template::hooks::captured();
    tokio::spawn(async move {
        let created = Box::pin(job.create(startup));
        #[cfg(test)]
        let outcome = match scoped {
            Some(scoped) => creation_template::hooks::HOOKS.scope(scoped, created).await,
            None => created.await,
        };
        #[cfg(not(test))]
        let outcome = created.await;
        let _ = result.send(outcome);
    });
    waiting
        .await
        .context("memory store creation worker stopped before cleanup")?
}

impl TemplateCreation {
    async fn create(mut self, startup: File) -> Result<(File, Outcome)> {
        let stage = files::ensure_private_directory(&self.stage)?;
        let created = Box::pin(creation_template::create_in(
            &self.root,
            &self.engine,
            &stage,
        ))
        .await;
        let judged = match created {
            Ok(Created::Copied {
                built,
                published,
                judged,
            }) => {
                if built {
                    tracing::info!(
                        published,
                        "built the store template for this engine and schema before copying it"
                    );
                }
                judged
            }
            Ok(Created::Unavailable(unavailable)) => {
                tracing::info!(
                    reason = ?unavailable,
                    "store template unavailable; creating the store without it"
                );
                self.set_aside(stage).await?;
                return Ok((startup, Outcome::Cold(self.marker_pause)));
            }
            // The build ran, or tried to run, the chain: a cold retry would
            // pay it again inside the same deadline. Its verdicts are against
            // its own unpublished bytes, so nothing is quarantined.
            Err(CreateError::Build(failure)) => {
                let error = failure
                    .into_error()
                    .context("build the memory store template");
                return match self.set_aside(stage).await {
                    Ok(()) => Err(error),
                    Err(preserve) => Err(error.context(format!(
                        "memory template stage preservation also failed: {preserve:#}"
                    ))),
                };
            }
            Err(CreateError::Use(failure)) => {
                tracing::warn!(
                    failure = %failure,
                    "store template unusable; creating the store without it"
                );
                self.set_aside(stage).await?;
                return Ok((startup, Outcome::Cold(self.marker_pause)));
            }
        };
        drop(stage);
        // Last: a stage without its identity record is an interrupted copy.
        #[cfg(test)]
        creation_template::hooks::identity_written(&self.stage);
        #[cfg(test)]
        let substituted = creation_template::hooks::stage_key();
        #[cfg(test)]
        let key = match substituted.as_deref() {
            Some(key) => key,
            None => creation_template::compiled_key(),
        };
        #[cfg(not(test))]
        let key = creation_template::compiled_key();
        if let Err(error) =
            crate::server::write_template_stage_identity(&self.stage, &self.project_scope, key)
        {
            tracing::warn!(
                error = %format!("{error:#}"),
                "store template stage identity unwritten; creating the store without it"
            );
            self.preserve().await?;
            return Ok((startup, Outcome::Cold(self.marker_pause)));
        }
        let base = &self.base;
        let make_options = |directory: PathBuf, read_only| ServerOptions {
            directory,
            read_only,
            ..base.clone()
        };
        let worker = stage_worker::StageWorker {
            make_options: &make_options,
            stage: &self.stage,
            parent: &self.parent,
            lifecycle_root: base.lifecycle_root.as_deref(),
            timeout: base.timeout,
            project_scope: &self.project_scope,
            legacy: None,
            #[cfg(test)]
            migration_hooks: None,
            #[cfg(test)]
            migrated_stage_pool_delay: None,
        };
        let mut progress = ProgressReporter::silent();
        match worker
            .adopt_and_mark(startup, &mut self.marker_pause, &mut progress)
            .await
        {
            Ok(lock) => Ok((lock, Outcome::Ready)),
            Err(error) => {
                if crate::server::TemplateVerdict::find(&error).is_some()
                    && let Some(judged) = judged
                {
                    creation_template::quarantine_after_adoption(&self.root, judged, &self.stage)
                        .await;
                }
                Err(error)
            }
        }
    }

    /// Set aside a stage the template path leaves without an engine start:
    /// remove it when nothing was copied into it, or preserve the copy
    /// remnant under `interrupted/`.
    async fn set_aside(&self, stage: Directory) -> Result<()> {
        // A stage that cannot be listed may hold a copy: preserve it rather
        // than fail the fallback. Only preservation itself can fail the open.
        let empty = match fs::read_dir(stage.path()) {
            Ok(mut entries) => entries.next().is_none(),
            Err(error) => {
                tracing::warn!(
                    stage = %self.stage.display(),
                    error = %error,
                    "a memory template stage could not be listed; preserving it"
                );
                false
            }
        };
        if !empty {
            drop(stage);
            return self.preserve().await;
        }
        if let Err(error) = stage.remove_tree() {
            // An empty stage left behind is preserved by the next open's
            // recovery; this open creates its store in another one.
            tracing::warn!(
                stage = %self.stage.display(),
                error = %error,
                "an empty memory template stage was left in place"
            );
        }
        Ok(())
    }

    async fn preserve(&self) -> Result<()> {
        preserve_unready_stage(
            &self.stage,
            &self.parent,
            self.base.lifecycle_root.as_deref(),
            self.base.timeout,
        )
        .await
        .context("preserve the interrupted memory template copy")
    }
}
