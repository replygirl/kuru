//! Compiled acceptance fixture access to the actual private supervisor protocol.
use super::*;

/// The compiled creator owns the real lifetime endpoint but sends no Request.
/// Its inherited output keeps the supervisor's completion observable after
/// creator termination without reopening either process by numeric PID.
pub(crate) async fn unconfigured(
    supervisor: &Path,
    directory: &Path,
    observer: &mut pipe::Pipe,
) -> Result<()> {
    use kuru_platform::windows::process::{StandardStream, inherited_stdio};
    let listener = pipe::PrivateListener::bind()?;
    let mut command = NativeSpawnSpec::new(supervisor.to_owned(), directory.to_owned());
    command.args = vec![
        "--internal-dolt-supervisor".into(),
        listener.address().to_owned(),
        "--fixture-unconfigured".into(),
    ];
    command.environment = crate::test_support::windows::environment()?;
    command.lifetime = Lifetime::TrustedSupervisor;
    command.console = Console::PrivateHidden;
    command.stdout = inherited_stdio(StandardStream::Output)?;
    let mut owner = Owner {
        child: Some(command.spawn().await?),
        lifetime: None,
        retained: None,
        reap_guard: Arc::new(StdMutex::new(None)),
        #[cfg(test)]
        reaped_observer: None,
    };
    let result = async {
        owner.lifetime = Some(
            listener
                .accept(owner.child.as_ref().unwrap(), Duration::from_secs(5))
                .await?,
        );
        ensure!(
            owner.child.as_mut().unwrap().try_wait()?.is_none(),
            "unconfigured supervisor exited before creator acknowledgment"
        );
        observer.write_all(b"READY\n").await?;
        observer.flush().await?;
        let mut byte = [0];
        timeout(Duration::from_secs(45), observer.read(&mut byte)).await??;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Every ordinary error/observer EOF closes the endpoint and awaits the
    // existing owner cleanup. Forced creator loss supplies that EOF through OS
    // handle closure instead; the supervisor's startup deadline remains intact.
    let stopped = finish_owner(&mut owner).await;
    result?;
    stopped
}

pub(crate) async fn partial_readiness(
    options: ServerOptions,
    observer: &mut pipe::Pipe,
) -> Result<()> {
    prepare_directory(&options.directory, false)?;
    let directory = fs::canonicalize(&options.directory)?;
    let request = Request {
        binary: options.binary,
        directory: directory.clone(),
        project_scope: options.project_scope,
        timeout_millis: options.timeout.as_millis().try_into()?,
        read_only: false,
        lifecycle_root: options.lifecycle_root,
    };
    let listener = pipe::PrivateListener::bind()?;
    let mut command = NativeSpawnSpec::new(options.supervisor, directory.clone());
    command.args = vec![
        "--internal-dolt-supervisor".into(),
        listener.address().to_owned(),
    ];
    command.environment = crate::test_support::windows::environment()?;
    command.lifetime = Lifetime::TrustedSupervisor;
    command.console = Console::PrivateHidden;
    let child = command.spawn().await?;
    let mut owner = Owner {
        child: Some(child),
        lifetime: None,
        retained: None,
        reap_guard: Arc::new(StdMutex::new(None)),
        #[cfg(test)]
        reaped_observer: None,
    };
    let result = async {
        owner.lifetime = Some(
            listener
                .accept(owner.child.as_ref().unwrap(), Duration::from_secs(5))
                .await?,
        );
        let channel = owner.lifetime.as_mut().unwrap();
        write_frame(channel, &request).await?;
        timeout(options.timeout, async {
            let length = channel.read_u32().await? as usize;
            ensure!(
                length > 9 && length <= RECORD_LIMIT,
                "invalid readiness length"
            );
            let mut prefix = [0; 9];
            channel.read_exact(&mut prefix).await?;
            ensure!(&prefix == b"{\"Ready\":", "supervisor did not become ready");
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        // Observe real initialized SQL independently while leaving the rest of
        // the frame unread. This does not imply its small server write blocked.
        let identity = load_identity(&directory, &request.project_scope)?
            .context("partial readiness has no initialized identity")?;
        ensure!(
            identity.initialized,
            "readiness preceded database initialization"
        );
        ensure!(
            live_endpoint(&directory, &identity, true).await?.is_some(),
            "readiness has no live authenticated endpoint"
        );
        observer.write_all(b"READY\n").await?;
        observer.flush().await?;
        let mut byte = [0];
        timeout(Duration::from_secs(45), observer.read(&mut byte)).await??;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Use production owner cleanup on every ordinary error/EOF. When the
    // creator is killed, OS handle closure instead supplies supervisor EOF.
    let stopped = finish_owner(&mut owner).await;
    result?;
    stopped
}
