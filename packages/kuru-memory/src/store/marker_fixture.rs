//! One private observation at the actual initial ready-marker publication.
//! Ordinary opens have no observer; this is not a configuration or fault API.
use super::*;
use tokio::sync::oneshot;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyMarkerObservation {
    pub after_marker: bool,
    pub stage: PathBuf,
    pub identity: [u8; 24],
    pub initial_revision: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Boundary {
    Before,
    After,
}

pub(super) struct ReadyMarkerPause {
    boundary: Boundary,
    reached: oneshot::Sender<ReadyMarkerObservation>,
    release: oneshot::Receiver<()>,
}

/// Consume only the selected one-shot observation. Cancellation is an error
/// inside initialization, so its existing pool/server cleanup still executes.
pub(super) async fn reach(
    pause: &mut Option<ReadyMarkerPause>,
    boundary: Boundary,
    stage: &Path,
    activation: &Activation,
) -> Result<()> {
    if !pause
        .as_ref()
        .is_some_and(|pause| pause.boundary == boundary)
    {
        return Ok(());
    }
    let pause = pause.take().expect("selected marker observation");
    let observation = ReadyMarkerObservation {
        after_marker: boundary == Boundary::After,
        stage: stage.to_owned(),
        identity: files::directory(stage)?.identity().to_bytes(),
        initial_revision: activation.initial_revision.clone(),
    };
    pause
        .reached
        .send(observation)
        .map_err(|_| anyhow::anyhow!("ready-marker observer disappeared before its observation"))?;
    tokio::time::timeout(Duration::from_secs(45), pause.release)
        .await
        .context("ready-marker observation release deadline exceeded")?
        .context("ready-marker observer closed before release")?;
    Ok(())
}

#[cfg(any(windows, test))]
pub(crate) fn prepare(
    options: OpenOptions,
    after_marker: bool,
) -> (
    oneshot::Receiver<ReadyMarkerObservation>,
    oneshot::Sender<()>,
    impl std::future::Future<Output = Result<MemoryStore>>,
) {
    let (reached, observation) = oneshot::channel();
    let (release, wait) = oneshot::channel();
    let boundary = if after_marker {
        Boundary::After
    } else {
        Boundary::Before
    };
    let pause = ReadyMarkerPause {
        boundary,
        reached,
        release: wait,
    };
    (observation, release, {
        let mut progress = crate::progress::ProgressReporter::silent();
        async move { MemoryStore::open_inner(options, None, None, Some(pause), &mut progress).await }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_marker_observer_finishes_real_initialization_cleanup_and_recovery() -> Result<()>
    {
        for after_marker in [false, true] {
            let root = crate::test_support::tempdir()?;
            let scope = format!("project/{}", "5".repeat(64));
            let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
            let (observation, release, opening) = prepare(options.clone(), after_marker);
            tokio::pin!(opening);
            let observed = tokio::select! {
                result = &mut opening => {
                    let store = result?;
                    store.close().await?;
                    anyhow::bail!("initialization completed without the requested marker observation");
                }
                observed = observation => observed?,
            };
            assert_eq!(observed.after_marker, after_marker);
            assert_eq!(observed.stage.join("ready.json").exists(), after_marker);
            assert!(!project_directory(root.path(), &scope)?.exists());
            let namespace = cfg!(windows).then(|| root.path().join("memory/lifecycles"));
            assert!(
                Server::quiescence_at(
                    &observed.stage,
                    namespace.as_deref(),
                    Duration::from_millis(20)
                )
                .await
                .is_err()
            );
            drop(release);
            let error = opening.await.unwrap_err();
            assert!(
                format!("{error:#}").contains("observer closed before release"),
                "{error:#}"
            );
            let preserved = root
                .path()
                .join("memory/interrupted")
                .join(observed.stage.file_name().unwrap());
            let stopped_stage = if after_marker {
                &observed.stage
            } else {
                &preserved
            };
            let lease =
                Server::quiescence_at(stopped_stage, namespace.as_deref(), Duration::from_secs(5))
                    .await?;
            assert_eq!(lease.directory.identity().to_bytes(), observed.identity);
            assert!(!observed.stage.join("endpoint.json").exists());
            drop(lease);
            let recovered = MemoryStore::open(options).await?;
            let active = project_directory(root.path(), &scope)?;
            if after_marker {
                assert_eq!(
                    files::directory(&active)?.identity().to_bytes(),
                    observed.identity
                );
                assert_eq!(recovered.revision().await?, observed.initial_revision);
            } else {
                assert_eq!(
                    files::directory(&preserved)?.identity().to_bytes(),
                    observed.identity
                );
                assert!(!preserved.join("ready.json").exists());
                assert_ne!(
                    files::directory(&active)?.identity().to_bytes(),
                    observed.identity
                );
            }
            recovered.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn released_marker_observation_activates_the_same_committed_store_once() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "4".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        let (observation, release, opening) = prepare(options.clone(), true);
        tokio::pin!(opening);
        let observed = tokio::select! {
            result = &mut opening => {
                let store = result?;
                store.close().await?;
                anyhow::bail!("initialization did not pause");
            }
            observed = observation => observed?,
        };
        release
            .send(())
            .map_err(|_| anyhow::anyhow!("pause lost its release receiver"))?;
        let store = opening.await?;
        assert_eq!(store.revision().await?, observed.initial_revision);
        let active = project_directory(root.path(), &scope)?;
        assert_eq!(
            files::directory(&active)?.identity().to_bytes(),
            observed.identity
        );
        assert!(!observed.stage.exists());
        store.close().await?;
        let reopened = MemoryStore::open(options).await?;
        assert_eq!(reopened.revision().await?, observed.initial_revision);
        assert_eq!(
            reopened
                .revisions(20)
                .await?
                .iter()
                .filter(|revision| revision.message == "Initialize Kuru memory schema 1")
                .count(),
            1
        );
        reopened.close().await?;
        Ok(())
    }
}
