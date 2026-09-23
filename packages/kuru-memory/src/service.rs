//! Private local protocol for the project memory owner.
//!
//! This module contains no provider, tool or conversation control operation.
//! Transport privacy excludes other OS users; a same-user process able to read
//! the private endpoint record is within the account's local authority.

#[cfg(test)]
use std::sync::Arc;
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) mod rpc;
pub use rpc::{ServiceCall, ServiceReply, ServiceRequest, ServiceResponse, ServiceValue};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 1;
pub const HANDSHAKE_LIMIT: usize = 16 * 1024;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const SERVICE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ATTACHMENTS: usize = 32;

/// Internal process entry used by both the ordinary executable and native
/// fixtures. Paths arrive as native OS arguments so non-UTF-8 names survive.
pub async fn service_entry(arguments: impl IntoIterator<Item = OsString>) -> Result<()> {
    let (project, options) = parse_service_arguments(arguments)?;
    ServiceOwner::open(options, &project).await?.serve().await
}

fn parse_service_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<(PathBuf, crate::store::OpenOptions)> {
    let mut arguments = arguments.into_iter();
    let project = PathBuf::from(arguments.next().context("missing service project path")?);
    let data = PathBuf::from(arguments.next().context("missing service data path")?);
    let scope = arguments
        .next()
        .context("missing service project scope")?
        .into_string()
        .map_err(|_| anyhow::anyhow!("service project scope is not UTF-8"))?;
    let dolt_binary = optional_path(arguments.next().context("missing service engine path")?);
    let cache_dir = optional_path(arguments.next().context("missing service cache path")?);
    let supervisor = optional_path(
        arguments
            .next()
            .context("missing service supervisor path")?,
    );
    let offline = match arguments
        .next()
        .context("missing service offline setting")?
        .to_str()
    {
        Some("0") => false,
        Some("1") => true,
        _ => bail!("invalid service offline setting"),
    };
    let startup_timeout_secs = arguments
        .next()
        .context("missing service startup timeout")?
        .to_str()
        .context("service startup timeout is not UTF-8")?
        .parse()?;
    ensure!(arguments.next().is_none(), "unexpected service argument");
    if let Some(path) = &supervisor {
        ensure!(
            path.is_absolute(),
            "service supervisor path must be absolute"
        );
    }
    let mut options = crate::store::OpenOptions::new(data, scope);
    options.config.dolt_binary = dolt_binary;
    options.config.cache_dir = cache_dir;
    options.config.offline = offline;
    options.config.startup_timeout_secs = startup_timeout_secs;
    options.supervisor = supervisor;
    Ok((project, options))
}

fn optional_path(argument: OsString) -> Option<PathBuf> {
    (argument != OsStr::new("-")).then(|| PathBuf::from(argument))
}

#[cfg(feature = "test-support")]
pub async fn client_fixture_entry(arguments: impl IntoIterator<Item = OsString>) -> Result<()> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    let ready = PathBuf::from(arguments.pop().context("missing fixture ready marker")?);
    let barrier = PathBuf::from(arguments.pop().context("missing fixture start barrier")?);
    let (project, options) = parse_service_arguments(arguments)?;
    std::fs::write(&ready, b"ready")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !barrier.exists() {
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service fixture start barrier was not released"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let executable = std::env::current_exe()?;
    let mut client = attach_or_start(&options, &project, &executable).await?;
    let result = client
        .call(ServiceCall::AppendMessage {
            namespace: "multiprocess-fixture".into(),
            message: kuru_core::Message::text("user", format!("{}", std::process::id())),
        })
        .await?;
    ensure!(matches!(result, ServiceValue::Unit));
    println!("{}", client.generation());
    Ok(())
}

/// Native Windows fixture: the caller places this starter in a containing
/// Job, then observes the owner through a distinct client after it exits.
#[cfg(all(windows, feature = "test-support"))]
pub async fn held_client_fixture_entry(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<()> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    let release = PathBuf::from(arguments.pop().context("missing fixture release marker")?);
    let ready = PathBuf::from(arguments.pop().context("missing fixture ready marker")?);
    let (project, options) = parse_service_arguments(arguments)?;
    let executable = std::env::current_exe()?;
    let mut client = attach_or_start(&options, &project, &executable).await?;
    ensure!(matches!(
        client
            .call(ServiceCall::AppendMessage {
                namespace: "starter-exit-fixture".into(),
                message: kuru_core::Message::text("user", "starter committed"),
            })
            .await?,
        ServiceValue::Unit
    ));
    std::fs::write(&ready, client.generation())?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !release.exists() {
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service starter release deadline exceeded"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

#[cfg(unix)]
pub type LocalStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type LocalStream = kuru_platform::windows::pipe::Pipe;

/// One live attachment. Keeping it open prevents the owner from idling out;
/// a failed exchange invalidates the stream rather than replaying a write.
pub struct ServiceAttachment {
    stream: Option<LocalStream>,
    authority: EndpointAuthority,
    locator: Option<AttachmentLocator>,
    last_fault: Option<rpc::ServiceFault>,
    #[cfg(test)]
    reply_pause: Option<Arc<rpc::ReplyPause>>,
}

#[derive(Clone)]
struct AttachmentLocator {
    data: PathBuf,
    address: String,
}

#[derive(Clone)]
pub(crate) struct AttachmentFactory {
    authority: EndpointAuthority,
    locator: AttachmentLocator,
}

impl AttachmentFactory {
    pub(crate) fn store_instance(&self) -> &str {
        &self.authority.store_instance
    }

    pub(crate) async fn connect(&self) -> Result<ServiceAttachment> {
        let mut stream = connect_local(
            &self.locator.data,
            &self.authority.project_scope,
            &self.locator.address,
            HANDSHAKE_TIMEOUT,
        )
        .await?;
        connect_handshake(&mut stream, &self.authority).await?;
        Ok(ServiceAttachment {
            stream: Some(stream),
            authority: self.authority.clone(),
            locator: Some(self.locator.clone()),
            last_fault: None,
            #[cfg(test)]
            reply_pause: None,
        })
    }
}

impl ServiceAttachment {
    pub(crate) fn store_instance(&self) -> &str {
        &self.authority.store_instance
    }

    pub fn generation(&self) -> &str {
        &self.authority.service_generation
    }

    pub(crate) fn has_complete_exchange(&self) -> bool {
        self.stream.is_some()
    }

    /// A generic storage failure can conceal a committed but unresolved
    /// operation; a complete wire reply alone is not mutation proof.
    pub(crate) fn has_definite_mutation_reply(&self) -> bool {
        self.has_complete_exchange()
            && !matches!(self.last_fault, Some(rpc::ServiceFault::StorageFailed))
    }

    pub async fn call(&mut self, call: ServiceCall) -> Result<ServiceValue> {
        self.call_with_id(uuid::Uuid::new_v4(), call).await
    }

    pub(crate) async fn call_with_id(
        &mut self,
        id: uuid::Uuid,
        call: ServiceCall,
    ) -> Result<ServiceValue> {
        let mut stream = self
            .stream
            .take()
            .context("memory service attachment is closed")?;
        self.last_fault = None;
        #[cfg(test)]
        let response = if let Some(pause) = self.reply_pause.take() {
            rpc::exchange_attached_with_id_paused(&mut stream, &self.authority, id, call, &pause)
                .await?
        } else {
            rpc::exchange_attached_with_id(&mut stream, &self.authority, id, call).await?
        };
        #[cfg(not(test))]
        let response =
            rpc::exchange_attached_with_id(&mut stream, &self.authority, id, call).await?;
        self.last_fault = match &response {
            rpc::ServiceResponse::Rejected(fault) => Some(*fault),
            rpc::ServiceResponse::Success(_) => None,
        };
        self.stream = Some(stream);
        rpc::resolve_response(response)
    }

    /// Open another authenticated attachment to the same service generation.
    /// Candidate and export handles use their own connection-owned state.
    pub async fn fork(&self) -> Result<Self> {
        self.factory()?.connect().await
    }

    pub(crate) fn factory(&self) -> Result<AttachmentFactory> {
        let locator = self
            .locator
            .as_ref()
            .context("fixture attachment cannot open another connection")?;
        Ok(AttachmentFactory {
            authority: self.authority.clone(),
            locator: locator.clone(),
        })
    }

    pub fn close(&mut self) {
        self.stream = None;
    }

    #[cfg(test)]
    pub(crate) fn pause_after_next_send(&mut self, pause: Arc<rpc::ReplyPause>) {
        self.reply_pause = Some(pause);
    }
}

/// Attach to a valid owner, or elect and start one while retaining a distinct
/// short start lock. Endpoint readiness is the authenticated private handshake;
/// the child holds the owner lock before publishing it.
pub async fn attach_or_start(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
) -> Result<ServiceAttachment> {
    ensure_project_scope(project, &options.project_scope)?;
    options.config.validate()?;
    ensure!(
        executable.is_absolute(),
        "service executable must be absolute"
    );
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(options.config.startup_timeout_secs);
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
        .await
        .context("attach before memory service start election")?
    {
        return Ok(attached);
    }
    let _start = loop {
        if let Some(lock) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Start,
        )? {
            break lock;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service election deadline exceeded"
        );
        if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
            .await
            .context("attach while waiting for memory service start election")?
        {
            return Ok(attached);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
        .await
        .context("attach after acquiring memory service start election")?
    {
        return Ok(attached);
    }
    loop {
        // A live owner may still be booting, or reaping Dolt after dropping
        // its endpoint. A busy lock never authorizes another spawn.
        if let Some(owner_probe) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            owner_probe.verify()?;
            drop(owner_probe);
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "existing memory service owner did not publish a valid endpoint"
        );
        if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
            .await
            .context("attach while the elected memory service owner is still active")?
        {
            return Ok(attached);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut child = ServiceProcess::new(
        spawn_service(options, project, executable, None)
            .await
            .context("spawn elected memory service owner")?,
    );
    loop {
        match try_attach(&options.data_dir, &options.project_scope, project).await {
            Ok(Some(attached)) => return Ok(attached),
            Ok(None) => {}
            Err(error) => {
                let child_state = match child.try_wait() {
                    Ok(Some(status)) => format!("exited with {status}"),
                    Ok(None) => "remained running".into(),
                    Err(status_error) => {
                        format!("status observation failed with {status_error}")
                    }
                };
                return Err(error).with_context(|| {
                    format!("attach after starting the elected memory service; child {child_state}")
                });
            }
        }
        if let Some(status) = child.try_wait()? {
            bail!("memory service exited before readiness: {status}");
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service readiness deadline exceeded"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A read-only inspection may attach to a published owner without electing or
/// starting one. The fallback remains an explicitly local read-only open.
pub(crate) async fn attach_existing(
    options: &crate::store::OpenOptions,
    project: &Path,
) -> Result<Option<ServiceAttachment>> {
    ensure_project_scope(project, &options.project_scope)?;
    options.config.validate()?;
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(options.config.startup_timeout_secs);
    loop {
        if let Some(attachment) =
            try_attach(&options.data_dir, &options.project_scope, project).await?
        {
            return Ok(Some(attachment));
        }
        if let Some(owner_probe) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            owner_probe.verify()?;
            return Ok(None);
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "active memory service did not publish a readable endpoint"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn try_attach(data: &Path, scope: &str, project: &Path) -> Result<Option<ServiceAttachment>> {
    let Some(record) = EndpointRecord::read(data, scope)? else {
        return Ok(None);
    };
    ensure!(
        record.authority.project_path == project_path_bytes(project),
        "memory service endpoint belongs to another project path"
    );
    let mut stream = match connect_local(data, scope, &record.address, HANDSHAKE_TIMEOUT).await {
        Ok(stream) => stream,
        Err(error)
            if error
                .chain()
                .filter_map(|cause| cause.downcast_ref::<io::Error>())
                .any(|cause| {
                    matches!(
                        cause.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    )
                }) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    connect_handshake(&mut stream, &record.authority).await?;
    Ok(Some(ServiceAttachment {
        stream: Some(stream),
        authority: record.authority,
        locator: Some(AttachmentLocator {
            data: data.to_owned(),
            address: record.address,
        }),
        last_fault: None,
        #[cfg(test)]
        reply_pause: None,
    }))
}

fn ensure_project_scope(project: &Path, scope: &str) -> Result<()> {
    ensure!(
        project.is_absolute() && std::fs::canonicalize(project)? == project && project.is_dir(),
        "memory service project path must be a canonical directory"
    );
    let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
    let expected = format!(
        "project/{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    ensure!(
        scope == expected,
        "memory service project scope does not match its canonical path"
    );
    Ok(())
}

fn service_arguments(options: &crate::store::OpenOptions, project: &Path) -> Vec<OsString> {
    fn path_or_dash(path: Option<&PathBuf>) -> OsString {
        path.map_or_else(|| OsString::from("-"), |path| path.as_os_str().to_owned())
    }
    vec![
        "--internal-memory-service".into(),
        project.as_os_str().to_owned(),
        options.data_dir.as_os_str().to_owned(),
        options.project_scope.clone().into(),
        path_or_dash(options.config.dolt_binary.as_ref()),
        path_or_dash(options.config.cache_dir.as_ref()),
        path_or_dash(options.supervisor.as_ref()),
        if options.config.offline { "1" } else { "0" }.into(),
        options.config.startup_timeout_secs.to_string().into(),
    ]
}

#[cfg(unix)]
async fn spawn_service(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    stderr: Option<File>,
) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(executable);
    command.args(service_arguments(options, project));
    command.current_dir(project);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(stderr.map_or_else(std::process::Stdio::null, std::process::Stdio::from));
    command.process_group(0);
    command.spawn().context("start project memory service")
}

#[cfg(windows)]
async fn spawn_service(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    stderr: Option<File>,
) -> Result<kuru_platform::windows::process::NativeChild> {
    use kuru_platform::windows::process::{Console, Lifetime, NativeSpawnSpec, Stdio};
    let mut command = NativeSpawnSpec::new(executable.to_owned(), project.to_owned());
    command.args = service_arguments(options, project);
    command.lifetime = Lifetime::IndependentService;
    command.console = Console::PrivateHidden;
    command.stderr = stderr.map_or(Stdio::Null, |file| Stdio::Handle(file.into()));
    let system = kuru_platform::windows::process::system_directory()?;
    let windows = system
        .parent()
        .context("Windows system directory has no parent")?;
    command.environment = vec![
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("WINDIR".into(), windows.as_os_str().into()),
        ("PATH".into(), system.into_os_string()),
    ];
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command
            .environment
            .push(("LLVM_PROFILE_FILE".into(), profile));
    }
    command.spawn().await.context(
        "start independent project memory service; a containing Windows Job must allow breakaway",
    )
}

#[cfg(unix)]
type ServiceChild = std::process::Child;
#[cfg(windows)]
type ServiceChild = kuru_platform::windows::process::NativeChild;

/// A retained child handle is installed immediately after spawn. Dropping a
/// cancelled starter still arranges process observation without terminating
/// the independently owned service.
struct ServiceProcess(Option<ServiceChild>);

impl ServiceProcess {
    fn new(child: ServiceChild) -> Self {
        Self(Some(child))
    }

    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.0.as_mut().expect("retained service child").try_wait()
    }
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        #[cfg(unix)]
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        #[cfg(windows)]
        tokio::spawn(async move {
            loop {
                match child.wait(Duration::from_secs(3600)).await {
                    Ok(_) => break,
                    Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
                    Err(_) => break,
                }
            }
        });
    }
}

pub struct ServiceListener {
    #[cfg(unix)]
    inner: kuru_platform::local_ipc::PrivateServiceListener,
    #[cfg(windows)]
    inner: kuru_platform::windows::pipe::PrivateServiceListener,
}

/// The service process owns this state after the short starter election lock
/// has been released. The caller must stop accepting and drain live handlers
/// before `close`; normal shutdown then reaps Dolt before retiring discovery.
pub struct ServiceOwner {
    lock: ServiceLock,
    store: crate::store::MemoryStore,
    listener: ServiceListener,
    record: EndpointRecord,
    data_dir: PathBuf,
    receipt_progress: std::sync::Arc<rpc::ReceiptProgress>,
}

impl ServiceOwner {
    pub async fn open(options: crate::store::OpenOptions, project_path: &Path) -> Result<Self> {
        ensure!(
            !options.read_only,
            "memory service owner must open writable storage"
        );
        ensure_project_scope(project_path, &options.project_scope)?;
        let lock = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )?
        .context("project already has a memory service owner; wait for its validated endpoint")?;
        lock.verify()?;
        let store = crate::store::MemoryStore::open(options.clone()).await?;
        let prepared = async {
            let (listener, address) =
                ServiceListener::bind(&options.data_dir, &options.project_scope)?;
            let record =
                EndpointRecord::for_store(project_path, &options.project_scope, &store, address)
                    .await?;
            record.publish(&options.data_dir, &lock)?;
            Ok::<_, anyhow::Error>((listener, record))
        }
        .await;
        let (listener, record) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                // A failed listener or publication must still reap Dolt while
                // this process retains its service-owner authority.
                if let Err(cleanup) = store.close().await {
                    return Err(error.context(format!(
                        "reap Dolt after memory service startup failed: {cleanup:#}"
                    )));
                }
                return Err(error);
            }
        };
        Ok(Self {
            lock,
            store,
            listener,
            record,
            data_dir: options.data_dir,
            receipt_progress: std::sync::Arc::new(rpc::ReceiptProgress::default()),
        })
    }

    pub fn authority(&self) -> &EndpointAuthority {
        &self.record.authority
    }

    pub async fn accept(&mut self, deadline: Duration) -> Result<LocalStream> {
        self.lock.verify()?;
        self.listener.accept(deadline).await
    }

    /// Run until the last client releases its connection and the idle grace
    /// elapses. Each connection has one generation-bound attachment; the store
    /// itself keeps reads concurrent and serializes short writes.
    pub async fn serve(mut self) -> Result<()> {
        let served = self.serve_until_idle().await;
        let closed = self.close().await;
        served.and(closed)
    }

    async fn serve_until_idle(&mut self) -> Result<()> {
        self.serve_until_idle_with(
            SERVICE_IDLE_TIMEOUT,
            SERVICE_IDLE_TIMEOUT + HANDSHAKE_TIMEOUT,
        )
        .await
    }

    async fn serve_until_idle_with(
        &mut self,
        idle_timeout: Duration,
        accept_timeout: Duration,
    ) -> Result<()> {
        let mut attachments = tokio::task::JoinSet::new();
        let frame_budget = std::sync::Arc::new(tokio::sync::Semaphore::new(rpc::FRAME_BUDGET_MIB));
        let retirement = std::sync::Arc::new(rpc::Retirement::default());
        loop {
            if let Err(error) = self.lock.verify() {
                abort_and_drain(&mut attachments).await;
                return Err(error);
            }
            if retirement.requested() {
                while let Some(completed) = attachments.join_next().await {
                    if let Err(error) = completed {
                        tracing::warn!(error = %error, "memory service retirement attachment failed");
                    }
                }
                break;
            }
            if attachments.is_empty() {
                tokio::select! {
                    biased;
                    accepted = self.listener.accept(accept_timeout) => {
                        self.attach(accepted?, &mut attachments, &frame_budget, &retirement);
                    }
                    _ = tokio::time::sleep(idle_timeout) => break,
                    _ = retirement.notified() => continue,
                }
            } else {
                tokio::select! {
                    accepted = self.listener.accept(accept_timeout) => {
                        match accepted {
                            Ok(stream) => self.attach(stream, &mut attachments, &frame_budget, &retirement),
                            Err(error) if is_accept_timeout(&error) => continue,
                            Err(error) => {
                                abort_and_drain(&mut attachments).await;
                                return Err(error);
                            }
                        }
                    }
                    completed = attachments.join_next() => {
                        if let Some(Err(error)) = completed {
                            tracing::warn!(error = %error, "memory service attachment task failed");
                        }
                    }
                    _ = retirement.notified() => continue,
                }
            }
        }
        Ok(())
    }

    fn attach(
        &self,
        stream: LocalStream,
        attachments: &mut tokio::task::JoinSet<()>,
        frame_budget: &std::sync::Arc<tokio::sync::Semaphore>,
        retirement: &std::sync::Arc<rpc::Retirement>,
    ) {
        if attachments.len() >= MAX_ATTACHMENTS {
            return;
        }
        let Some(retained) = retirement.attached() else {
            return;
        };
        let authority = self.record.authority.clone();
        let store = self.store.clone();
        let frame_budget = frame_budget.clone();
        let retirement = retirement.clone();
        let receipt_progress = self.receipt_progress.clone();
        attachments.spawn(async move {
            let _retained = retained;
            let mut stream = stream;
            if let Err(error) = rpc::serve_attached(
                &mut stream,
                &authority,
                &store,
                frame_budget,
                retirement,
                receipt_progress,
            )
            .await
                && !is_peer_closed(&error)
            {
                tracing::warn!(error = %error, "memory service attachment ended with an error");
            }
        });
    }

    /// Close only after accepted requests and client attachments have drained.
    /// A crash instead leaves a stale record which a successor reconciles only
    /// after obtaining owner and existing lifecycle authority.
    pub async fn close(self) -> Result<()> {
        let Self {
            lock,
            store,
            listener,
            record,
            data_dir,
            receipt_progress: _,
        } = self;
        drop(listener);
        store.close().await?;
        record.retire(&data_dir, &lock)?;
        lock.verify()?;
        Ok(())
    }
}

fn is_accept_timeout(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<tokio::time::error::Elapsed>()
            || cause
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::TimedOut)
    })
}

async fn abort_and_drain(attachments: &mut tokio::task::JoinSet<()>) {
    attachments.abort_all();
    while attachments.join_next().await.is_some() {}
}

impl ServiceListener {
    pub fn bind(data_dir: &Path, scope: &str) -> Result<(Self, String)> {
        #[cfg(windows)]
        let directory =
            crate::files::ensure_private_directory(&EndpointRecord::directory(data_dir, scope)?)?;
        #[cfg(unix)]
        {
            let _ = data_dir;
            let directory =
                kuru_platform::local_ipc::prepare_short_directory(unix_socket_locator(scope)?)?;
            let address = format!("s-{}", uuid::Uuid::new_v4().simple());
            let listener = kuru_platform::local_ipc::PrivateServiceListener::bind_at(
                directory,
                OsStr::new(&address),
            )?;
            Ok((Self { inner: listener }, address))
        }
        #[cfg(windows)]
        {
            directory.revalidate()?;
            let listener = kuru_platform::windows::pipe::PrivateServiceListener::bind()?;
            let address = listener.address().to_string_lossy().into_owned();
            Ok((Self { inner: listener }, address))
        }
    }

    pub async fn accept(&mut self, deadline: Duration) -> Result<LocalStream> {
        #[cfg(unix)]
        {
            tokio::time::timeout(deadline, self.inner.accept())
                .await
                .context("memory service accept deadline exceeded")?
                .map_err(Into::into)
        }
        #[cfg(windows)]
        {
            self.inner.accept(deadline).await.map_err(Into::into)
        }
    }
}

pub async fn connect_local(
    data_dir: &Path,
    scope: &str,
    address: &str,
    deadline: Duration,
) -> Result<LocalStream> {
    #[cfg(unix)]
    let directory = {
        let _ = data_dir;
        kuru_platform::local_ipc::open_short_directory(unix_socket_locator(scope)?)?
    };
    #[cfg(windows)]
    let directory = crate::files::open_directory(
        &EndpointRecord::directory(data_dir, scope)?,
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    #[cfg(unix)]
    {
        tokio::time::timeout(
            deadline,
            kuru_platform::local_ipc::connect(&directory, OsStr::new(address)),
        )
        .await
        .context("memory service connect deadline exceeded")?
        .map_err(Into::into)
    }
    #[cfg(windows)]
    {
        directory.revalidate()?;
        kuru_platform::windows::pipe::connect(OsStr::new(address), deadline)
            .await
            .map_err(Into::into)
    }
}

#[cfg(unix)]
fn unix_socket_locator(scope: &str) -> Result<&str> {
    let hash = scope
        .strip_prefix("project/")
        .context("memory project scope must start with project/")?;
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "memory project scope must contain a lowercase SHA256 digest"
    );
    Ok(&hash[..24])
}

/// The start lock arbitrates publishing a new owner. It is released after
/// readiness. The distinct owner lock remains held until Dolt is reaped.
#[derive(Clone, Copy, Debug)]
pub enum ServiceLockKind {
    Start,
    Owner,
}

pub struct ServiceLock {
    directory: Directory,
    name: OsString,
    file: File,
    kind: ServiceLockKind,
}

/// Retains both election and owner authority while an explicit maintenance
/// operation inspects or moves this project's storage. An active service must
/// retire before this permit can be acquired; a new starter cannot elect until
/// the permit is dropped.
pub(crate) struct MaintenancePermit {
    _start: ServiceLock,
    _owner: ServiceLock,
}

pub(crate) async fn acquire_maintenance_permit(
    options: &crate::store::OpenOptions,
) -> Result<MaintenancePermit> {
    options.config.validate()?;
    ensure!(
        !options.read_only,
        "memory maintenance requires writable options"
    );
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(options.config.startup_timeout_secs);
    let start = loop {
        if let Some(lock) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Start,
        )? {
            break lock;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory maintenance election deadline exceeded"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut retirement_requested = false;
    let mut busy_observations = 0;
    let owner = loop {
        if let Some(lock) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            break lock;
        }
        if !retirement_requested {
            match tokio::time::timeout_at(deadline, request_idle_retirement(options))
                .await
                .context("memory maintenance owner-response deadline exceeded")??
            {
                Some(true) => retirement_requested = true,
                Some(false) => {
                    busy_observations += 1;
                    ensure!(
                        busy_observations < 10,
                        "memory service has active clients; close them before maintenance"
                    );
                }
                None => {}
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service owner is still active; maintenance cannot proceed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    start.verify()?;
    owner.verify()?;
    Ok(MaintenancePermit {
        _start: start,
        _owner: owner,
    })
}

/// The caller holds the start gate, so a successful idle retirement cannot
/// race a replacement election while the owner reaps Dolt. A missing/stale
/// endpoint is a wait condition; a valid owner with live clients refuses.
async fn request_idle_retirement(options: &crate::store::OpenOptions) -> Result<Option<bool>> {
    let Some(record) = EndpointRecord::read(&options.data_dir, &options.project_scope)? else {
        return Ok(None);
    };
    let stream = match connect_local(
        &options.data_dir,
        &options.project_scope,
        &record.address,
        HANDSHAKE_TIMEOUT,
    )
    .await
    {
        Ok(stream) => stream,
        Err(error)
            if error
                .chain()
                .filter_map(|cause| cause.downcast_ref::<io::Error>())
                .any(|cause| {
                    matches!(
                        cause.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    )
                }) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error).context("connect to memory service for maintenance"),
    };
    let mut attachment = ServiceAttachment {
        stream: Some(stream),
        authority: record.authority,
        locator: None,
        last_fault: None,
        #[cfg(test)]
        reply_pause: None,
    };
    connect_handshake(
        attachment.stream.as_mut().expect("new maintenance stream"),
        &attachment.authority,
    )
    .await?;
    let result = attachment.call(ServiceCall::RetireIfIdle).await;
    attachment.close();
    match result? {
        ServiceValue::Retirement { accepted } => Ok(Some(accepted)),
        _ => bail!("memory service returned the wrong maintenance response"),
    }
}

impl ServiceLock {
    /// A busy owner lock means an existing process may still be cleaning up.
    /// Never remove its lockfile or infer takeover authority from a PID.
    pub fn try_acquire(
        data_dir: &Path,
        scope: &str,
        kind: ServiceLockKind,
    ) -> Result<Option<Self>> {
        let project = crate::store::project_directory(data_dir, scope)?;
        let hash = project.file_name().context("project store has no name")?;
        let locks = data_dir.join("memory/locks");
        let directory = Directory::ensure_private(&locks)
            .context("service lock directory must be owner-private")?;
        let suffix = match kind {
            ServiceLockKind::Start => ".service-start.lock",
            ServiceLockKind::Owner => ".service-owner.lock",
        };
        let name = OsString::from(format!("{}{}", hash.to_string_lossy(), suffix));
        let file = directory.lock_file(&name)?;
        match file.try_lock() {
            Ok(()) => {
                directory.verify(&name, &file)?;
                Ok(Some(Self {
                    directory,
                    name,
                    file,
                    kind,
                }))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(error).context("acquire project service lock"),
        }
    }

    pub fn verify(&self) -> Result<()> {
        self.directory.verify(&self.name, &self.file)?;
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointRecord {
    pub authority: EndpointAuthority,
    /// Unix socket basename or Windows private pipe address, validated by the
    /// native connector before use. This is a discovery hint, not authority.
    pub address: String,
}

impl EndpointRecord {
    pub async fn for_store(
        project_path: &Path,
        scope: &str,
        store: &crate::store::MemoryStore,
        address: String,
    ) -> Result<Self> {
        ensure_project_scope(project_path, scope)?;
        let generation = uuid::Uuid::new_v4().to_string();
        let secret = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        Ok(Self {
            authority: EndpointAuthority {
                version: ProtocolVersion::CURRENT,
                project_path: project_path_bytes(project_path),
                project_scope: scope.to_owned(),
                store_instance: store.service_instance().to_owned(),
                service_generation: generation,
                connection_secret: secret,
                schema_version: store.schema_version().await?,
            },
            address,
        })
    }

    fn directory(data_dir: &Path, scope: &str) -> Result<PathBuf> {
        let project = crate::store::project_directory(data_dir, scope)?;
        let hash = project.file_name().context("project store has no name")?;
        Ok(data_dir.join("memory/services").join(hash))
    }

    fn path(data_dir: &Path, scope: &str) -> Result<PathBuf> {
        Ok(Self::directory(data_dir, scope)?.join("endpoint.json"))
    }

    pub fn publish(&self, data_dir: &Path, owner: &ServiceLock) -> Result<()> {
        ensure!(
            matches!(owner.kind, ServiceLockKind::Owner),
            "endpoint publication requires service owner authority"
        );
        owner.verify()?;
        let path = Self::path(data_dir, &self.authority.project_scope)?;
        crate::files::ensure_private_directory(
            path.parent().context("service endpoint has no parent")?,
        )?;
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= HANDSHAKE_LIMIT,
            "service endpoint record exceeds limit"
        );
        crate::files::write(&path, &bytes)?;
        owner.verify()?;
        Ok(())
    }

    pub fn read(data_dir: &Path, scope: &str) -> Result<Option<Self>> {
        let path = Self::path(data_dir, scope)?;
        let bytes = match crate::files::read_bytes(&path, HANDSHAKE_LIMIT as u64) {
            Ok(bytes) => bytes,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("read private memory service endpoint"),
        };
        let record: Self =
            serde_json::from_slice(&bytes).context("decode private memory service endpoint")?;
        ensure!(
            record.authority.project_scope == scope,
            "service endpoint project mismatch"
        );
        Ok(Some(record))
    }

    /// Retire only the held record of this owner generation. A replacement is
    /// never removed, even if the old process finishes a delayed cleanup.
    pub fn retire(&self, data_dir: &Path, owner: &ServiceLock) -> Result<()> {
        ensure!(
            matches!(owner.kind, ServiceLockKind::Owner),
            "endpoint retirement requires service owner authority"
        );
        owner.verify()?;
        let directory = crate::files::open_directory(
            &Self::directory(data_dir, &self.authority.project_scope)?,
            Privacy::OwnerOnly,
            // Windows needs the retained endpoint handle to share deletion with
            // the checked DELETE handle opened by remove_file. The owner lock
            // and exact generation/secret check still guard this name.
            NameRetention::Movable,
        )?;
        let name = OsStr::new("endpoint.json");
        let mut file = match directory.read(name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take((HANDSHAKE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= HANDSHAKE_LIMIT,
            "service endpoint record exceeds limit"
        );
        directory.verify(name, &file)?;
        let current: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            current.authority.service_generation == self.authority.service_generation
                && current.authority.connection_secret == self.authority.connection_secret,
            "service endpoint generation changed during retirement"
        );
        directory.remove_file(name, file)?;
        owner.verify()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
    };
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHello {
    pub version: ProtocolVersion,
    /// Exact bytes of the already-canonical project path on this OS.
    pub project_path: Vec<u8>,
    pub project_scope: String,
    pub store_instance: String,
    pub service_generation: String,
    pub connection_secret: String,
    pub schema_version: i32,
}

impl std::fmt::Debug for ClientHello {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientHello")
            .field("version", &self.version)
            .field("project_scope", &self.project_scope)
            .field("store_instance", &self.store_instance)
            .field("service_generation", &self.service_generation)
            .field("schema_version", &self.schema_version)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointAuthority {
    pub version: ProtocolVersion,
    pub project_path: Vec<u8>,
    pub project_scope: String,
    pub store_instance: String,
    pub service_generation: String,
    pub connection_secret: String,
    pub schema_version: i32,
}

impl EndpointAuthority {
    pub fn hello(&self) -> ClientHello {
        ClientHello {
            version: self.version,
            project_path: self.project_path.clone(),
            project_scope: self.project_scope.clone(),
            store_instance: self.store_instance.clone(),
            service_generation: self.service_generation.clone(),
            connection_secret: self.connection_secret.clone(),
            schema_version: self.schema_version,
        }
    }

    pub fn verify(&self, hello: &ClientHello) -> std::result::Result<(), HandshakeRejection> {
        if hello.version.major != self.version.major || hello.version.minor > self.version.minor {
            return Err(HandshakeRejection::Protocol);
        }
        if hello.project_path != self.project_path || hello.project_scope != self.project_scope {
            return Err(HandshakeRejection::Project);
        }
        if hello.store_instance != self.store_instance {
            return Err(HandshakeRejection::StoreInstance);
        }
        if hello.service_generation != self.service_generation {
            return Err(HandshakeRejection::Generation);
        }
        if !same_secret(
            hello.connection_secret.as_bytes(),
            self.connection_secret.as_bytes(),
        ) {
            return Err(HandshakeRejection::Authentication);
        }
        if hello.schema_version != self.schema_version {
            return Err(HandshakeRejection::Schema);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandshakeRejection {
    Protocol,
    Project,
    StoreInstance,
    Generation,
    Authentication,
    Schema,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HandshakeReply {
    Accepted {
        version: ProtocolVersion,
        service_generation: String,
    },
    Rejected {
        reason: HandshakeRejection,
    },
}

/// Admit one connection before its handler can see a storage request. A
/// rejected connection receives only a fixed diagnostic code, never authority
/// material from the endpoint record.
pub async fn accept_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
) -> Result<std::result::Result<(), HandshakeRejection>> {
    let hello: ClientHello = read_frame(stream, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT).await?;
    let decision = authority.verify(&hello);
    let reply = match decision {
        Ok(()) => HandshakeReply::Accepted {
            version: authority.version,
            service_generation: authority.service_generation.clone(),
        },
        Err(reason) => HandshakeReply::Rejected { reason },
    };
    write_frame(stream, &reply, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT).await?;
    Ok(decision)
}

/// Verify that the peer accepted exactly the generation requested. The
/// connection is discarded by the caller on any failure.
pub async fn connect_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
) -> Result<()> {
    ensure!(
        authority.version.major == PROTOCOL_MAJOR && authority.version.minor >= PROTOCOL_MINOR,
        "memory service protocol is incompatible; close the active Kuru session or use its matching version"
    );
    write_frame(
        stream,
        &authority.hello(),
        HANDSHAKE_LIMIT,
        HANDSHAKE_TIMEOUT,
    )
    .await?;
    let reply: HandshakeReply = read_frame(stream, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT).await?;
    match reply {
        HandshakeReply::Accepted {
            version,
            service_generation,
        } if version.major == authority.version.major
            && version.minor >= authority.version.minor
            && service_generation == authority.service_generation =>
        {
            Ok(())
        }
        HandshakeReply::Accepted { .. } => {
            bail!("memory service accepted an incompatible generation or protocol")
        }
        HandshakeReply::Rejected { reason } => bail!("{}", reason.diagnostic()),
    }
}

impl HandshakeRejection {
    pub const fn diagnostic(self) -> &'static str {
        match self {
            Self::Protocol => {
                "memory service protocol is incompatible; close the active Kuru session or use its matching version"
            }
            Self::Project => "memory service belongs to another project",
            Self::StoreInstance => {
                "memory service store identity changed; reconnect after the active owner exits"
            }
            Self::Generation => {
                "memory service generation changed; reconnect using its current endpoint"
            }
            Self::Authentication => "memory service connection was not authenticated",
            Self::Schema => {
                "memory service schema is incompatible; close the active Kuru session or use its matching version"
            }
        }
    }
}

fn same_secret(left: &[u8], right: &[u8]) -> bool {
    let mut different = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        different |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    different == 0
}

/// Encode one bounded JSON frame. A failed or cancelled write makes this
/// connection unusable; callers must close it instead of replaying blindly.
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
    limit: usize,
    deadline: Duration,
) -> Result<()> {
    let payload = serde_json::to_vec(value).context("encode memory service frame")?;
    ensure!(
        !payload.is_empty() && payload.len() <= limit && payload.len() <= u32::MAX as usize,
        "memory service frame exceeds its limit"
    );
    let length = u32::try_from(payload.len())?.to_be_bytes();
    tokio::time::timeout(deadline, async {
        writer.write_all(&length).await?;
        writer.write_all(&payload).await?;
        writer.flush().await
    })
    .await
    .context("memory service frame write deadline exceeded")?
    .context("write memory service frame")
}

/// Decode one bounded JSON frame without allocating the announced payload
/// until its length has passed the fixed caller-supplied limit.
pub async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
    limit: usize,
    deadline: Duration,
) -> Result<T> {
    tokio::time::timeout(deadline, async {
        let mut header = [0u8; 4];
        reader.read_exact(&mut header).await?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > limit {
            bail!("memory service frame exceeds its limit");
        }
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await?;
        serde_json::from_slice(&payload).context("decode memory service frame")
    })
    .await
    .context("memory service frame read deadline exceeded")?
}

/// Project identity uses native encoded bytes so non-UTF-8 Unix paths are not
/// silently normalized or rejected by a JSON string conversion.
pub fn project_path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

pub fn is_peer_closed(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::UnexpectedEof
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use tokio::io::duplex;

    const FIXTURE_DIAGNOSTIC_TAIL_BYTES: u64 = 4 * 1024;

    #[derive(Default)]
    struct FixtureAttachObservations {
        missing_endpoint: usize,
        published_transport_unavailable: usize,
        pipe_connect_timeout: usize,
    }

    impl std::fmt::Display for FixtureAttachObservations {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                formatter,
                "missing endpoint: {}; published transport unavailable: {}; private pipe connect timeout: {}",
                self.missing_endpoint,
                self.published_transport_unavailable,
                self.pipe_connect_timeout
            )
        }
    }

    fn fixture_diagnostic_tail(file: &mut File) -> Result<String> {
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(
            length.saturating_sub(FIXTURE_DIAGNOSTIC_TAIL_BYTES),
        ))?;
        let mut bytes = Vec::new();
        file.take(FIXTURE_DIAGNOSTIC_TAIL_BYTES)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            Ok("fixture service stderr remained empty".into())
        } else {
            Ok(format!(
                "fixture service stderr tail: {}",
                String::from_utf8_lossy(&bytes)
            ))
        }
    }

    fn fixture_readiness_error(
        options: &crate::store::OpenOptions,
        diagnostic: &mut File,
        stage: &'static str,
        outcome: String,
        observations: &FixtureAttachObservations,
    ) -> Result<anyhow::Error> {
        let stderr = fixture_diagnostic_tail(diagnostic)?;
        let error = anyhow::anyhow!(outcome)
            .context(format!("readiness observations: {observations}; {stderr}"));
        Ok(crate::test_support::fixture_startup_error(options, error).context(stage))
    }

    /// A crash fixture retains the exact spawned child. Early test failure
    /// terminates that handle; ServiceProcess then installs its native reaper.
    struct KillServiceOnDrop(ServiceProcess);

    impl KillServiceOnDrop {
        fn terminate(&mut self) -> Result<()> {
            let child = self
                .0
                .0
                .as_mut()
                .context("service child was already reaped")?;
            #[cfg(unix)]
            child.kill().context("terminate fixture service process")?;
            #[cfg(windows)]
            child
                .terminate()
                .context("terminate fixture service process")?;
            Ok(())
        }
    }

    impl Drop for KillServiceOnDrop {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.terminate();
            }
        }
    }

    /// A published endpoint does not prove that this particular attempt has
    /// obtained a Windows pipe instance. Keep that fixture-only readiness
    /// observation inside the caller's named outer deadline; product
    /// attachment still treats a connect timeout as an error and never
    /// silently retries an ambiguous request.
    async fn try_attach_fixture_stage(
        data: &Path,
        scope: &str,
        project: &Path,
        stage: &'static str,
        observations: &mut FixtureAttachObservations,
    ) -> Result<Option<ServiceAttachment>> {
        let endpoint_published = EndpointRecord::read(data, scope)?.is_some();
        match try_attach(data, scope, project).await {
            Ok(Some(attached)) => Ok(Some(attached)),
            Ok(None) => {
                if endpoint_published {
                    observations.published_transport_unavailable += 1;
                } else {
                    observations.missing_endpoint += 1;
                }
                Ok(None)
            }
            #[cfg(windows)]
            Err(error) if is_private_pipe_connect_timeout(&error) => {
                observations.pipe_connect_timeout += 1;
                Ok(None)
            }
            Err(error) => Err(error).with_context(|| format!("{stage} attachment failed")),
        }
    }

    #[cfg(windows)]
    fn is_private_pipe_connect_timeout(error: &anyhow::Error) -> bool {
        error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<io::Error>())
            .any(|cause| {
                cause.kind() == io::ErrorKind::TimedOut
                    && cause.to_string() == "private pipe connect timed out"
            })
    }

    #[test]
    fn crash_fixture_diagnostics_are_distinct_and_bounded() -> Result<()> {
        let observations = FixtureAttachObservations {
            missing_endpoint: 7,
            published_transport_unavailable: 3,
            pipe_connect_timeout: 2,
        };
        assert_eq!(
            observations.to_string(),
            "missing endpoint: 7; published transport unavailable: 3; private pipe connect timeout: 2"
        );

        let root = crate::test_support::tempdir()?;
        let path = root.path().join("fixture.stderr");
        let mut file = File::options()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)?;
        file.write_all(&vec![b'x'; FIXTURE_DIAGNOSTIC_TAIL_BYTES as usize + 1])?;
        file.write_all(b"terminal fixture cause")?;
        file.flush()?;
        let tail = fixture_diagnostic_tail(&mut file)?;
        assert!(tail.starts_with("fixture service stderr tail: "));
        assert!(tail.ends_with("terminal fixture cause"));
        assert_eq!(
            tail.len(),
            "fixture service stderr tail: ".len() + FIXTURE_DIAGNOSTIC_TAIL_BYTES as usize
        );
        Ok(())
    }

    #[tokio::test]
    async fn inspection_waits_for_a_booting_owner_without_starting_dolt() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let data = root.path().join("private");
        let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        options.read_only = true;
        options.config.startup_timeout_secs = 1;
        let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let error =
            tokio::time::timeout(Duration::from_secs(3), attach_existing(&options, &project))
                .await
                .context("inspection did not respect the owner startup deadline")?
                .err()
                .context("inspection bypassed an unpublished live owner")?;
        ensure!(
            format!("{error:#}").contains("did not publish a readable endpoint"),
            "inspection did not identify the booting owner: {error:#}"
        );
        drop(owner);
        ensure!(
            attach_existing(&options, &project).await?.is_none(),
            "inspection treated a retired owner as live"
        );
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn stale_pipe_does_not_replace_a_live_service_owner() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let data = root.path().join("private");
        let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        options.config.startup_timeout_secs = 1;
        let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let mut endpoint_authority = authority();
        endpoint_authority.project_path = project_path_bytes(&project);
        endpoint_authority.project_scope = scope.clone();
        let endpoint = EndpointRecord {
            authority: endpoint_authority,
            address: format!(r"\\.\pipe\kuru-{}", uuid::Uuid::new_v4()),
        };
        endpoint.publish(&data, &owner)?;

        let executable = project.join("must-not-spawn.exe");
        let error = tokio::time::timeout(
            HANDSHAKE_TIMEOUT.saturating_mul(3),
            attach_or_start(&options, &project, &executable),
        )
        .await
        .context("live-owner fixture exceeded its outer deadline")?
        .err()
        .context("stale transport authorized replacing a live owner")?;
        ensure!(
            format!("{error:#}").contains("existing memory service owner did not publish"),
            "live owner refusal lost its authoritative stage: {error:#}"
        );
        ensure!(
            EndpointRecord::read(&data, &scope)?.is_some(),
            "refused replacement retired the live owner's endpoint"
        );
        ensure!(
            ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_none(),
            "refused replacement displaced the live owner's lock"
        );
        drop(owner);
        endpoint.retire(
            &data,
            &ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
                .context("fixture did not reacquire owner lock for cleanup")?,
        )?;
        Ok(())
    }

    #[tokio::test]
    async fn purge_refuses_a_live_service_owner_before_writing_intent() -> Result<()> {
        let data = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "a".repeat(64));
        let mut options = crate::OpenOptions::new(data.path().to_owned(), scope.clone());
        options.config.startup_timeout_secs = 1;
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let error =
            tokio::time::timeout(Duration::from_secs(3), crate::MemoryStore::purge(options))
                .await
                .context("purge did not respect its owner deadline")?
                .expect_err("purge must not run while the service owns the store");
        ensure!(
            format!("{error:#}").contains("memory service owner is still active"),
            "purge did not explain the live owner: {error:#}"
        );
        ensure!(
            !data
                .path()
                .join("memory/controls")
                .join(format!("{}.json", "a".repeat(64)))
                .exists(),
            "refused purge wrote durable intent"
        );
        drop(owner);
        Ok(())
    }

    #[tokio::test]
    async fn maintenance_retires_only_an_idle_owner_and_holds_election() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(crate::store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let served = tokio::spawn(owner.serve());
            let mut client = try_attach(&data, &scope, &project)
                .await?
                .context("fixture owner did not accept a client")?;

            let refused = tokio::time::timeout(
                Duration::from_secs(5),
                crate::MemoryStore::purge(options.clone()),
            )
            .await
            .context("busy owner maintenance refusal exceeded five seconds")?
            .expect_err("maintenance admitted another live client");
            ensure!(
                format!("{refused:#}").contains("active clients"),
                "busy owner refusal lacked client context: {refused:#}"
            );
            ensure!(
                !data
                    .join("memory/controls")
                    .join(format!("{}.json", &scope["project/".len()..]))
                    .exists(),
                "refused purge wrote durable intent"
            );
            ensure!(
                matches!(
                    client.call(ServiceCall::Revision).await?,
                    ServiceValue::Revision(_)
                ),
                "refused maintenance disturbed the live client"
            );
            client.close();

            let permit = tokio::time::timeout(
                Duration::from_secs(20),
                acquire_maintenance_permit(&options),
            )
            .await
            .context("idle owner was not retired before its 30-second grace period")??;
            ensure!(
                ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?.is_none(),
                "maintenance did not retain the starter election gate"
            );
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "retired owner still published an endpoint"
            );
            tokio::time::timeout(Duration::from_secs(5), served)
                .await
                .context("retired owner did not finish reaping")???;
            drop(permit);
            ensure!(
                ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?.is_some(),
                "maintenance did not release the election gate"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("idle-owner maintenance fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[cfg(windows)]
    fn windows_starter_fixture(
        project: &Path,
        options: &crate::store::OpenOptions,
        executable: &Path,
        ready: &Path,
        release: &Path,
        lifetime: kuru_platform::windows::process::Lifetime,
    ) -> kuru_platform::windows::process::NativeSpawnSpec {
        use kuru_platform::windows::process::NativeSpawnSpec;
        let mut command = NativeSpawnSpec::new(executable.to_owned(), project.to_owned());
        command.args = service_arguments(options, project);
        command.args[0] = "--internal-memory-service-held-client-fixture".into();
        command.args.push(ready.as_os_str().to_owned());
        command.args.push(release.as_os_str().to_owned());
        command.lifetime = lifetime;
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.environment.push(("SystemRoot".into(), root));
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command
                .environment
                .push(("LLVM_PROFILE_FILE".into(), profile));
        }
        command
    }

    #[cfg(windows)]
    fn windows_service_fixture() -> Result<(
        crate::test_support::TempDir,
        PathBuf,
        crate::store::OpenOptions,
        PathBuf,
    )> {
        let root = crate::test_support::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let mut options = crate::store::OpenOptions::new(root.path().join("private"), scope);
        options.config.cache_dir = Some(crate::store::test_cache());
        options.config.offline = true;
        let executable = crate::store::test_supervisor()?;
        options.supervisor = Some(executable.clone());
        Ok((root, project, options, executable))
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn starter_job_exit_preserves_independent_owner_and_surviving_client() -> Result<()> {
        use kuru_platform::windows::process::Lifetime;
        tokio::time::timeout(Duration::from_secs(110), async {
            let (root, project, options, executable) = windows_service_fixture()?;
            let ready = root.path().join("starter-ready");
            let release = root.path().join("starter-release");
            let _gate = crate::spawn_gate::spawning().await;
            let mut starter = windows_starter_fixture(
                &project,
                &options,
                &executable,
                &ready,
                &release,
                Lifetime::FixtureBreakawayJob,
            )
            .spawn()
            .await?;
            let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(40);
            while !ready.exists() {
                if let Some(status) = starter.try_wait()? {
                    bail!("contained memory starter exited before readiness: {status}");
                }
                ensure!(
                    tokio::time::Instant::now() < ready_deadline,
                    "contained memory starter did not publish readiness"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let generation = std::fs::read_to_string(&ready)?;
            let mut survivor = try_attach(&options.data_dir, &options.project_scope, &project)
                .await?
                .context("independent survivor could not attach to starter's owner")?;
            ensure!(
                survivor.generation() == generation,
                "survivor generation changed"
            );
            std::fs::write(&release, b"release")?;
            ensure!(
                starter.wait(Duration::from_secs(10)).await?.success(),
                "contained starter failed while exiting"
            );
            drop(starter); // closes the containing kill-on-close Job
            ensure!(matches!(
                survivor
                    .call(ServiceCall::AppendMessage {
                        namespace: "starter-exit-fixture".into(),
                        message: kuru_core::Message::text("user", "survivor committed"),
                    })
                    .await?,
                ServiceValue::Unit
            ));
            let ServiceValue::HistoryWindow(window) = survivor
                .call(ServiceCall::HistoryWindow {
                    namespace: "starter-exit-fixture".into(),
                    limit: 4,
                })
                .await?
            else {
                bail!("surviving client received the wrong history result");
            };
            ensure!(
                window.total_rows == 2 && window.messages.len() == 2,
                "surviving client lost a committed row"
            );
            drop(survivor);
            let idle_deadline =
                tokio::time::Instant::now() + SERVICE_IDLE_TIMEOUT + Duration::from_secs(20);
            while EndpointRecord::read(&options.data_dir, &options.project_scope)?.is_some() {
                ensure!(
                    tokio::time::Instant::now() < idle_deadline,
                    "independent owner did not retire before fixture cleanup"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            while ServiceLock::try_acquire(
                &options.data_dir,
                &options.project_scope,
                ServiceLockKind::Owner,
            )?
            .is_none()
            {
                ensure!(
                    tokio::time::Instant::now() < idle_deadline,
                    "independent owner did not release its lifecycle lock after Dolt reap"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("Windows independent owner fixture exceeded 110 seconds")??;
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn denying_job_rejects_independent_owner_before_publication() -> Result<()> {
        use kuru_platform::windows::process::{Lifetime, Stdio};
        use tokio::io::AsyncReadExt;
        tokio::time::timeout(Duration::from_secs(40), async {
            let (root, project, options, executable) = windows_service_fixture()?;
            let ready = root.path().join("denied-ready");
            let release = root.path().join("denied-release");
            let _gate = crate::spawn_gate::spawning().await;
            let mut command = windows_starter_fixture(
                &project,
                &options,
                &executable,
                &ready,
                &release,
                Lifetime::OwnedJob,
            );
            command.stderr = Stdio::Pipe;
            let mut starter = command.spawn().await?;
            let mut stderr = starter.take_stderr().context("missing starter stderr")?;
            let drain = async move {
                let mut diagnostic = Vec::new();
                let mut truncated = false;
                let mut block = [0u8; 4096];
                loop {
                    let count = stderr.read(&mut block).await?;
                    if count == 0 {
                        break;
                    }
                    let retained = (16 * 1024usize).saturating_sub(diagnostic.len()).min(count);
                    diagnostic.extend_from_slice(&block[..retained]);
                    truncated |= retained < count;
                }
                Ok::<_, anyhow::Error>((
                    String::from_utf8_lossy(&diagnostic).into_owned(),
                    truncated,
                ))
            };
            let (status, diagnostic) = tokio::join!(starter.wait(Duration::from_secs(30)), drain);
            let status = status?;
            ensure!(
                !status.success(),
                "denied breakaway unexpectedly started owner"
            );
            let (diagnostic, truncated) = diagnostic?;
            ensure!(!truncated, "denied breakaway diagnostic exceeded 16 KiB");
            ensure!(
                diagnostic.contains("containing Windows Job must allow breakaway"),
                "denied breakaway lacked containment diagnosis: {diagnostic}"
            );
            drop(starter);
            ensure!(!ready.exists(), "denied starter published readiness");
            ensure!(
                EndpointRecord::read(&options.data_dir, &options.project_scope)?.is_none(),
                "denied owner published an endpoint"
            );
            ensure!(
                ServiceLock::try_acquire(
                    &options.data_dir,
                    &options.project_scope,
                    ServiceLockKind::Owner,
                )?
                .is_some(),
                "denied owner retained the lifecycle lock"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("Windows denied-breakaway fixture exceeded 40 seconds")??;
        Ok(())
    }

    fn authority() -> EndpointAuthority {
        EndpointAuthority {
            version: ProtocolVersion::CURRENT,
            project_path: b"/private/project".to_vec(),
            project_scope: "scope".into(),
            store_instance: "store".into(),
            service_generation: "generation".into(),
            connection_secret: "test-secret".into(),
            schema_version: 4,
        }
    }

    #[test]
    fn handshake_rejects_each_authority_mismatch_without_leaking_secret() {
        let authority = authority();
        let mut hello = authority.hello();
        assert!(authority.verify(&hello).is_ok());
        hello.version.major += 1;
        assert_eq!(authority.verify(&hello), Err(HandshakeRejection::Protocol));
        hello = authority.hello();
        hello.project_path.push(0);
        assert_eq!(authority.verify(&hello), Err(HandshakeRejection::Project));
        hello = authority.hello();
        hello.store_instance.push('x');
        assert_eq!(
            authority.verify(&hello),
            Err(HandshakeRejection::StoreInstance)
        );
        hello = authority.hello();
        hello.service_generation.push('x');
        assert_eq!(
            authority.verify(&hello),
            Err(HandshakeRejection::Generation)
        );
        hello = authority.hello();
        hello.connection_secret.push('x');
        assert_eq!(
            authority.verify(&hello),
            Err(HandshakeRejection::Authentication)
        );
        hello = authority.hello();
        hello.schema_version += 1;
        assert_eq!(authority.verify(&hello), Err(HandshakeRejection::Schema));
        let debug = format!("{hello:?}");
        assert!(!debug.contains("test-secret"));
        for rejection in [
            HandshakeRejection::Protocol,
            HandshakeRejection::Project,
            HandshakeRejection::StoreInstance,
            HandshakeRejection::Generation,
            HandshakeRejection::Authentication,
            HandshakeRejection::Schema,
        ] {
            assert!(!rejection.diagnostic().contains("test-secret"));
        }
    }

    #[tokio::test]
    async fn client_rejects_an_older_owner_before_sending_new_operations() -> Result<()> {
        let (mut client, _old_owner) = duplex(1024);
        let mut old = authority();
        old.version.minor -= 1;
        let error = connect_handshake(&mut client, &old).await.unwrap_err();
        ensure!(
            format!("{error:#}").contains("protocol is incompatible"),
            "older owner had no actionable compatibility result: {error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn bounded_frames_round_trip_and_reject_oversize_before_payload() {
        let (mut client, mut server) = duplex(1024);
        write_frame(
            &mut client,
            &authority().hello(),
            HANDSHAKE_LIMIT,
            HANDSHAKE_TIMEOUT,
        )
        .await
        .unwrap();
        let hello: ClientHello = read_frame(&mut server, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT)
            .await
            .unwrap();
        assert!(authority().verify(&hello).is_ok());

        client.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let error = read_frame::<_, ClientHello>(&mut server, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("exceeds its limit"));
    }

    #[tokio::test]
    async fn handshake_admits_matching_generation_and_rejects_other_project() {
        let expected = authority();
        let (mut client, mut server) = duplex(1024);
        let server_authority = expected.clone();
        let server_task = tokio::spawn(async move {
            accept_handshake(&mut server, &server_authority)
                .await
                .unwrap()
        });
        tokio::time::timeout(HANDSHAKE_TIMEOUT, connect_handshake(&mut client, &expected))
            .await
            .expect("matching handshake deadline")
            .unwrap();
        assert_eq!(server_task.await.unwrap(), Ok(()));

        let (mut client, mut server) = duplex(1024);
        let server_authority = expected.clone();
        let server_task = tokio::spawn(async move {
            accept_handshake(&mut server, &server_authority)
                .await
                .unwrap()
        });
        let mut wrong_project = expected;
        wrong_project.project_scope.push('x');
        let error = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            connect_handshake(&mut client, &wrong_project),
        )
        .await
        .expect("rejected handshake deadline")
        .unwrap_err();
        assert!(format!("{error:#}").contains("another project"));
        assert_eq!(server_task.await.unwrap(), Err(HandshakeRejection::Project));
    }

    #[test]
    fn start_and_owner_locks_have_independent_retained_authority() {
        let _gate = crate::spawn_gate::locking();
        let data = tempfile::tempdir().unwrap();
        let scope = format!("project/{}", "a".repeat(64));
        let start = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Start)
            .unwrap()
            .unwrap();
        assert!(
            ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Start)
                .unwrap()
                .is_none()
        );
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
            .unwrap()
            .unwrap();
        start.verify().unwrap();
        owner.verify().unwrap();
        drop(start);
        assert!(
            ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Start)
                .unwrap()
                .is_some()
        );
        assert!(
            ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
                .unwrap()
                .is_none()
        );
        drop(owner);
        assert!(
            ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn endpoint_retirement_preserves_a_changed_generation() {
        let _gate = crate::spawn_gate::locking();
        let data = tempfile::tempdir().unwrap();
        let scope = format!("project/{}", "b".repeat(64));
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
            .unwrap()
            .unwrap();
        let mut first = EndpointRecord {
            authority: authority(),
            address: "socket-first".into(),
        };
        first.authority.project_scope = scope.clone();
        first.publish(data.path(), &owner).unwrap();
        assert_eq!(
            EndpointRecord::read(data.path(), &scope)
                .unwrap()
                .unwrap()
                .address,
            "socket-first"
        );
        let mut later = first.clone();
        later.authority.service_generation = "later".into();
        later.address = "socket-later".into();
        later.publish(data.path(), &owner).unwrap();
        assert!(first.retire(data.path(), &owner).is_err());
        assert_eq!(
            EndpointRecord::read(data.path(), &scope)
                .unwrap()
                .unwrap()
                .address,
            "socket-later"
        );
        later.retire(data.path(), &owner).unwrap();
        assert!(EndpointRecord::read(data.path(), &scope).unwrap().is_none());
    }

    #[tokio::test]
    async fn native_listener_authenticates_before_any_operation() {
        let data = tempfile::tempdir().unwrap();
        let deep_data = data
            .path()
            .join("deep".repeat(16))
            .join("nested".repeat(12));
        Directory::ensure_private(&deep_data).unwrap();
        let scope = format!("project/{}", "c".repeat(64));
        let (mut listener, address) = ServiceListener::bind(&deep_data, &scope).unwrap();
        let mut expected = authority();
        expected.project_scope = scope.clone();
        let server_authority = expected.clone();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept(HANDSHAKE_TIMEOUT).await.unwrap();
            accept_handshake(&mut stream, &server_authority)
                .await
                .unwrap()
        });
        let mut client = connect_local(&deep_data, &scope, &address, HANDSHAKE_TIMEOUT)
            .await
            .unwrap();
        connect_handshake(&mut client, &expected).await.unwrap();
        assert_eq!(
            tokio::time::timeout(HANDSHAKE_TIMEOUT, server)
                .await
                .expect("native handshake server deadline")
                .unwrap(),
            Ok(())
        );
    }

    #[tokio::test]
    async fn cancelled_client_call_invalidates_its_connection() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let data = tempfile::tempdir()?;
            let scope = format!("project/{}", "d".repeat(64));
            let (mut listener, address) = ServiceListener::bind(data.path(), &scope)?;
            let mut expected = authority();
            expected.project_scope = scope.clone();
            let server_authority = expected.clone();
            let (received, ready) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut stream = listener.accept(HANDSHAKE_TIMEOUT).await?;
                ensure!(
                    accept_handshake(&mut stream, &server_authority)
                        .await?
                        .is_ok(),
                    "fixture handshake rejected"
                );
                let _: ServiceRequest = read_frame(&mut stream, 1024, HANDSHAKE_TIMEOUT).await?;
                let _ = received.send(());
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok::<(), anyhow::Error>(())
            });
            let mut stream =
                connect_local(data.path(), &scope, &address, HANDSHAKE_TIMEOUT).await?;
            connect_handshake(&mut stream, &expected).await?;
            let mut attachment = ServiceAttachment {
                stream: Some(stream),
                authority: expected,
                locator: None,
                last_fault: None,
                #[cfg(test)]
                reply_pause: None,
            };
            let mut call = Box::pin(attachment.call(ServiceCall::Revision));
            tokio::select! {
                result = &mut call => bail!("client call unexpectedly completed: {result:?}"),
                ready = ready => ready.context("fixture server did not receive request")?,
            }
            drop(call);
            ensure!(
                attachment.stream.is_none(),
                "cancelled call reused an uncertain stream"
            );
            ensure!(attachment.call(ServiceCall::Revision).await.is_err());
            server.await??;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("cancelled memory service call fixture exceeded 10 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn invalid_project_is_rejected_before_creating_store_state() -> Result<()> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("missing-project");
        let data = root.path().join("private");
        let options =
            crate::store::OpenOptions::new(data.clone(), format!("project/{}", "e".repeat(64)));
        ensure!(ServiceOwner::open(options, &project).await.is_err());
        ensure!(!data.exists(), "invalid project created memory state");
        Ok(())
    }

    #[tokio::test]
    async fn owner_reaps_real_dolt_before_retiring_endpoint_and_lock() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(crate::store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            ensure!(
                ServiceOwner::open(options, &project).await.is_err(),
                "second service owner must be rejected before another engine opens"
            );
            let record = EndpointRecord::read(&data, &scope)?
                .context("published service endpoint is missing")?;
            ensure!(
                record.authority.service_generation == owner.authority().service_generation,
                "published endpoint generation differs from retained owner"
            );
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut accepted = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let (client_result, server_result) = tokio::join!(
                rpc::request_one(
                    &mut client,
                    &record.authority,
                    ServiceCall::AppendMessage {
                        namespace: "owner-fixture".into(),
                        message: kuru_core::Message::text("user", "persisted"),
                    },
                ),
                rpc::serve_one(&mut accepted, owner.authority(), &owner.store),
            );
            ensure!(
                matches!(client_result?, ServiceValue::Unit),
                "service append returned the wrong response"
            );
            server_result?;
            drop(accepted);
            drop(client);
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut accepted = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let (client_result, server_result) = tokio::join!(
                rpc::request_one(
                    &mut client,
                    &record.authority,
                    ServiceCall::HistoryWindow {
                        namespace: "owner-fixture".into(),
                        limit: 4,
                    },
                ),
                rpc::serve_one(&mut accepted, owner.authority(), &owner.store),
            );
            let ServiceValue::HistoryWindow(window) = client_result? else {
                bail!("service history returned the wrong response");
            };
            ensure!(
                window.total_rows == 1 && window.messages[0].plain_text() == Some("persisted"),
                "service did not retain typed message history"
            );
            server_result?;
            drop(accepted);
            drop(client);
            owner.close().await?;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "closed owner retained its endpoint record"
            );
            ensure!(
                ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_some(),
                "closed owner retained its election lock"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("real memory service owner fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_reply_receipt_survives_sibling_write_and_owner_restart() -> Result<()> {
        use tokio::io::AsyncWriteExt;
        use uuid::Uuid;

        tokio::time::timeout(Duration::from_secs(90), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(crate::store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            let original = owner.authority().clone();
            let record = EndpointRecord::read(&data, &scope)?.context("missing owner endpoint")?;
            let id = Uuid::new_v4();
            let first = ServiceCall::AppendMessage {
                namespace: "lost-reply-fixture".into(),
                message: kuru_core::Message::text("user", "first committed"),
            };
            let (method, argument_digest) = first
                .unit_receipt_fingerprint("main")?
                .context("append has no logical receipt")?;
            let request = rpc::ServiceRequest::with_id(&original.service_generation, id, first);
            let body = serde_json::to_vec(&request)?;
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut accepted = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let sent = async {
                connect_handshake(&mut client, &original).await?;
                client.write_all(&(body.len() as u32).to_be_bytes()).await?;
                client.write_all(&body).await?;
                client.flush().await?;
                drop(client); // never inspect the accepted write's reply
                Ok::<(), anyhow::Error>(())
            };
            let (sent, _) =
                tokio::join!(sent, rpc::serve_one(&mut accepted, &original, &owner.store),);
            sent?;
            drop(accepted);

            let mut sibling =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut sibling_server = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let (sibling_reply, sibling_served) = tokio::join!(
                rpc::request_one(
                    &mut sibling,
                    &original,
                    ServiceCall::AppendMessage {
                        namespace: "lost-reply-fixture".into(),
                        message: kuru_core::Message::text("assistant", "sibling committed"),
                    },
                ),
                rpc::serve_one(&mut sibling_server, &original, &owner.store),
            );
            ensure!(matches!(sibling_reply?, ServiceValue::Unit));
            sibling_served?;
            drop(sibling_server);
            drop(sibling);

            let query = |original_id| ServiceCall::Outcome {
                original_id,
                original_generation: original.service_generation.clone(),
                view: "main".into(),
                method: method.into(),
                argument_digest: argument_digest.clone(),
            };
            let absent_id = Uuid::new_v4();
            for (query_id, expected) in [
                (id, rpc::OutcomeStatus::Committed),
                (absent_id, rpc::OutcomeStatus::StillUncertain),
            ] {
                let mut outcome =
                    connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
                let mut outcome_server = owner.accept(HANDSHAKE_TIMEOUT).await?;
                let (reported, served) = tokio::join!(
                    rpc::request_one(&mut outcome, &original, query(query_id)),
                    rpc::serve_one(&mut outcome_server, &original, &owner.store),
                );
                ensure!(matches!(reported?, ServiceValue::Outcome(status) if status == expected));
                served?;
                drop(outcome_server);
                drop(outcome);
            }
            owner.close().await?;

            let mut successor = ServiceOwner::open(options, &project).await?;
            ensure!(
                successor.authority().service_generation != original.service_generation,
                "owner restart retained its prior generation"
            );
            let current = successor.authority().clone();
            let current_record =
                EndpointRecord::read(&data, &scope)?.context("missing successor endpoint")?;
            for (query_id, expected) in [
                (id, rpc::OutcomeStatus::Committed),
                (absent_id, rpc::OutcomeStatus::Absent),
            ] {
                let mut client =
                    connect_local(&data, &scope, &current_record.address, HANDSHAKE_TIMEOUT)
                        .await?;
                let mut accepted = successor.accept(HANDSHAKE_TIMEOUT).await?;
                let (reported, served) = tokio::join!(
                    rpc::request_one(&mut client, &current, query(query_id)),
                    rpc::serve_one(&mut accepted, &current, &successor.store),
                );
                ensure!(matches!(reported?, ServiceValue::Outcome(status) if status == expected));
                served?;
                drop(accepted);
                drop(client);
            }
            successor.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("lost-reply receipt and owner restart fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn registered_in_flight_write_cannot_be_reported_absent() -> Result<()> {
        use uuid::Uuid;

        async fn query(
            owner: &mut ServiceOwner,
            data: &Path,
            scope: &str,
            authority: &EndpointAuthority,
            progress: &std::sync::Arc<rpc::ReceiptProgress>,
            call: ServiceCall,
        ) -> Result<ServiceValue> {
            let mut client =
                connect_local(data, scope, &owner.record.address, HANDSHAKE_TIMEOUT).await?;
            let mut accepted = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let request = async {
                connect_handshake(&mut client, authority).await?;
                rpc::resolve_response(rpc::exchange_attached(&mut client, authority, call).await?)
            };
            let (reply, served) = tokio::join!(
                request,
                rpc::serve_one_with_progress(&mut accepted, authority, &owner.store, progress),
            );
            served?;
            reply
        }

        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let data = root.path().join("private");
        let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        options.config.cache_dir = Some(crate::store::test_cache());
        options.config.offline = true;
        options.supervisor = Some(crate::store::test_supervisor()?);
        let _gate = crate::spawn_gate::spawning().await;
        let mut owner = ServiceOwner::open(options, &project).await?;
        let authority = owner.authority().clone();
        let progress = owner.receipt_progress.clone();
        let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
        progress.pause_next(pause.clone());

        let id = Uuid::new_v4();
        let write = ServiceCall::AppendMessage {
            namespace: "in-flight-outcome".into(),
            message: kuru_core::Message::text("user", "accepted once"),
        };
        let (method, argument_digest) = write
            .unit_receipt_fingerprint("main")?
            .context("append has no logical receipt")?;
        let outcome = || ServiceCall::Outcome {
            original_id: id,
            original_generation: authority.service_generation.clone(),
            view: "main".into(),
            method: method.into(),
            argument_digest: argument_digest.clone(),
        };
        let mut writer_client =
            connect_local(&data, &scope, &owner.record.address, HANDSHAKE_TIMEOUT).await?;
        let mut writer_server = owner.accept(HANDSHAKE_TIMEOUT).await?;
        let client_authority = authority.clone();
        let mut writer = tokio::spawn(async move {
            connect_handshake(&mut writer_client, &client_authority).await?;
            rpc::resolve_response(
                rpc::exchange_attached_with_id(&mut writer_client, &client_authority, id, write)
                    .await?,
            )
        });
        let server_authority = authority.clone();
        let server_store = owner.store.clone();
        let server_progress = progress.clone();
        let mut served = tokio::spawn(async move {
            rpc::serve_one_with_progress(
                &mut writer_server,
                &server_authority,
                &server_store,
                &server_progress,
            )
            .await
        });

        let tested = tokio::time::timeout(Duration::from_secs(40), async {
            tokio::time::timeout(Duration::from_secs(5), pause.entered.notified())
                .await
                .context("write did not register its receipt before dispatch")?;
            ensure!(
                matches!(
                    query(&mut owner, &data, &scope, &authority, &progress, outcome()).await?,
                    ServiceValue::Outcome(rpc::OutcomeStatus::InFlight)
                ),
                "registered in-flight write was not reported as in flight"
            );
            pause.release.notify_one();
            let (write_result, server_result) =
                tokio::time::timeout(Duration::from_secs(10), async {
                    tokio::join!(&mut writer, &mut served)
                })
                .await
                .context("registered write did not settle after release")?;
            ensure!(matches!(write_result??, ServiceValue::Unit));
            server_result??;
            ensure!(
                matches!(
                    query(&mut owner, &data, &scope, &authority, &progress, outcome()).await?,
                    ServiceValue::Outcome(rpc::OutcomeStatus::Committed)
                ),
                "settled write lost its committed receipt"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await;
        pause.release.notify_one();
        if !writer.is_finished() {
            writer.abort();
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut writer).await;
        }
        if !served.is_finished() {
            served.abort();
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut served).await;
        }
        let closed = tokio::time::timeout(Duration::from_secs(10), owner.close())
            .await
            .context("in-flight outcome fixture owner did not reap")?;
        tested.context("in-flight outcome fixture exceeded 40 seconds")??;
        closed?;
        Ok(())
    }

    #[tokio::test]
    async fn crashed_owner_retains_accepted_receipt_after_sibling_write() -> Result<()> {
        use tokio::io::AsyncWriteExt;

        tokio::time::timeout(Duration::from_secs(120), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let executable = crate::store::test_supervisor()?;
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(executable.clone());
            let _gate = crate::spawn_gate::spawning().await;
            let diagnostic_path = root.path().join("crash-service.stderr");
            let mut diagnostic = File::options()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&diagnostic_path)?;
            let mut process = KillServiceOnDrop(ServiceProcess::new(
                spawn_service(
                    &options,
                    &project,
                    &executable,
                    Some(diagnostic.try_clone()?),
                )
                .await?,
            ));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            let mut observations = FixtureAttachObservations::default();
            let mut original = loop {
                if let Some(attached) = try_attach_fixture_stage(
                    &data,
                    &scope,
                    &project,
                    "initial owner readiness",
                    &mut observations,
                )
                .await?
                {
                    break attached;
                }
                if let Some(status) = process.0.try_wait()? {
                    return Err(fixture_readiness_error(
                        &options,
                        &mut diagnostic,
                        "crash fixture service did not publish a usable endpoint",
                        format!("fixture service exited before readiness: {status}"),
                        &observations,
                    )?);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(fixture_readiness_error(
                        &options,
                        &mut diagnostic,
                        "crash fixture service did not publish a usable endpoint",
                        "memory supervisor readiness deadline exceeded".into(),
                        &observations,
                    )?);
                }
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(20)),
                )
                .await;
            };
            let generation = original.generation().to_owned();
            let request_id = uuid::Uuid::new_v4();
            let first = ServiceCall::AppendMessage {
                namespace: "crashed-owner-receipt".into(),
                message: kuru_core::Message::text("user", "accepted before crash"),
            };
            let (method, argument_digest) = first
                .unit_receipt_fingerprint("main")?
                .context("append has no logical receipt")?;
            let request = rpc::ServiceRequest::with_id(&generation, request_id, first);
            let body = serde_json::to_vec(&request)?;
            let mut stream = original
                .stream
                .take()
                .context("authenticated fixture attachment was closed")?;
            tokio::time::timeout(Duration::from_secs(5), async {
                stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
                stream.write_all(&body).await?;
                stream.flush().await?;
                Ok::<(), anyhow::Error>(())
            })
            .await
            .context("accepted fixture frame write exceeded five seconds")??;
            drop(stream); // the caller never reads the mutation reply
            drop(original);

            let sibling_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            let mut sibling_observations = FixtureAttachObservations::default();
            let mut sibling = loop {
                if let Some(attached) = try_attach_fixture_stage(
                    &data,
                    &scope,
                    &project,
                    "original-owner sibling readiness",
                    &mut sibling_observations,
                )
                .await?
                {
                    break attached;
                }
                if let Some(status) = process.0.try_wait()? {
                    return Err(fixture_readiness_error(
                        &options,
                        &mut diagnostic,
                        "sibling did not attach to the original owner",
                        format!("fixture service exited before sibling readiness: {status}"),
                        &sibling_observations,
                    )?);
                }
                if tokio::time::Instant::now() >= sibling_deadline {
                    return Err(fixture_readiness_error(
                        &options,
                        &mut diagnostic,
                        "sibling did not attach to the original owner",
                        "memory supervisor readiness deadline exceeded".into(),
                        &sibling_observations,
                    )?);
                }
                tokio::time::sleep_until(
                    sibling_deadline.min(tokio::time::Instant::now() + Duration::from_millis(20)),
                )
                .await;
            };
            ensure!(sibling.generation() == generation);
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let ServiceValue::HistoryWindow(window) = sibling
                        .call(ServiceCall::HistoryWindow {
                            namespace: "crashed-owner-receipt".into(),
                            limit: 4,
                        })
                        .await?
                    else {
                        bail!("sibling history returned the wrong response")
                    };
                    if window.total_rows == 1 {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("sibling could not observe the accepted durable write")??;
            ensure!(matches!(
                sibling
                    .call(ServiceCall::AppendMessage {
                        namespace: "crashed-owner-receipt".into(),
                        message: kuru_core::Message::text("assistant", "sibling before crash"),
                    })
                    .await?,
                ServiceValue::Unit
            ));
            drop(sibling);
            ensure!(
                process.0.try_wait()?.is_none(),
                "fixture owner exited before the deliberate crash"
            );
            process.terminate()?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if process.0.try_wait()?.is_some() {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("crashed service process was not reaped")??;

            let mut successor = attach_or_start(&options, &project, &executable)
                .await
                .context("successor election and readiness after crashed endpoint retirement")?;
            ensure!(successor.generation() != generation);
            let query = |original_id| ServiceCall::Outcome {
                original_id,
                original_generation: generation.clone(),
                view: "main".into(),
                method: method.into(),
                argument_digest: argument_digest.clone(),
            };
            ensure!(matches!(
                successor
                    .call(query(request_id))
                    .await
                    .context("successor could not recover the accepted receipt")?,
                ServiceValue::Outcome(rpc::OutcomeStatus::Committed)
            ));
            ensure!(matches!(
                successor
                    .call(query(uuid::Uuid::new_v4()))
                    .await
                    .context("successor could not classify an unrelated request")?,
                ServiceValue::Outcome(rpc::OutcomeStatus::Absent)
            ));
            ensure!(matches!(
                successor
                    .call_with_id(
                        request_id,
                        ServiceCall::AppendMessage {
                            namespace: "crashed-owner-receipt".into(),
                            message: kuru_core::Message::text("user", "accepted before crash"),
                        },
                    )
                    .await
                    .context("successor could not retry the exact accepted request")?,
                ServiceValue::Unit
            ));
            ensure!(
                successor
                    .call_with_id(
                        request_id,
                        ServiceCall::AppendMessage {
                            namespace: "crashed-owner-receipt".into(),
                            message: kuru_core::Message::text("user", "changed retry"),
                        },
                    )
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("conflicts"),
                "changed retry after owner crash reused the original receipt"
            );
            let ServiceValue::HistoryWindow(window) = successor
                .call(ServiceCall::HistoryWindow {
                    namespace: "crashed-owner-receipt".into(),
                    limit: 4,
                })
                .await
                .context("successor could not read history after receipt recovery")?
            else {
                bail!("successor history returned the wrong response")
            };
            ensure!(window.total_rows == 2 && window.messages.len() == 2);
            drop(successor);
            let permit = acquire_maintenance_permit(&options)
                .await
                .context("successor did not retire for fixture maintenance")?;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("crashed-owner receipt fixture exceeded 120 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_usage_reply_uses_natural_key_after_sibling_write_and_restart() -> Result<()> {
        use kuru_core::{InvocationStart, UsagePhase};
        use tokio::io::AsyncWriteExt;
        use uuid::Uuid;

        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let options = crate::test_support::open_options(data.clone(), scope.clone())?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            let original = owner.authority().clone();
            let record = EndpointRecord::read(&data, &scope)?.context("missing owner endpoint")?;
            let start = InvocationStart {
                session_id: "proof-session".into(),
                invocation_id: "proof-invocation".into(),
                operation_id: "proof-turn".into(),
                phase: UsagePhase::Speak,
                actor_id: "speaker".into(),
                route: "responses".into(),
                model: "model".into(),
                price_at_invocation: None,
            };
            let ledger = owner.store.usage_ledger()?;
            ledger.mark_new_session(&start.session_id).await?;
            drop(ledger);
            let proof = rpc::LedgerOperation::Admit {
                start: Box::new(start.clone()),
            }
            .proof()?
            .context("admit has no natural-key proof")?;
            let id = Uuid::new_v4();
            let request = rpc::ServiceRequest::with_id(
                &original.service_generation,
                id,
                ServiceCall::Ledger {
                    operation: Box::new(rpc::LedgerOperation::Admit {
                        start: Box::new(start.clone()),
                    }),
                },
            );
            let body = serde_json::to_vec(&request)?;
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut server = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let sent = async {
                connect_handshake(&mut client, &original).await?;
                client.write_all(&(body.len() as u32).to_be_bytes()).await?;
                client.write_all(&body).await?;
                client.flush().await?;
                drop(client);
                Ok::<(), anyhow::Error>(())
            };
            let (sent, _) =
                tokio::join!(sent, rpc::serve_one(&mut server, &original, &owner.store));
            sent?;
            drop(server);

            let sibling = owner.store.usage_ledger()?;
            sibling
                .admit(InvocationStart {
                    invocation_id: "sibling-invocation".into(),
                    ..start.clone()
                })
                .await?;
            drop(sibling);
            let query = |original_id, proof| ServiceCall::LedgerOutcome {
                original_id,
                original_generation: original.service_generation.clone(),
                proof,
            };
            let missing = crate::store::UsageProof::admit(&InvocationStart {
                invocation_id: "missing-invocation".into(),
                ..start.clone()
            })?;
            for (query_id, candidate_proof, expected) in [
                (id, proof.clone(), rpc::OutcomeStatus::Committed),
                (
                    Uuid::new_v4(),
                    missing.clone(),
                    rpc::OutcomeStatus::StillUncertain,
                ),
            ] {
                let mut client =
                    connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
                let mut server = owner.accept(HANDSHAKE_TIMEOUT).await?;
                let (response, served) = tokio::join!(
                    rpc::request_one(&mut client, &original, query(query_id, candidate_proof)),
                    rpc::serve_one(&mut server, &original, &owner.store),
                );
                ensure!(matches!(response?, ServiceValue::Outcome(status) if status == expected));
                served?;
                drop(server);
                drop(client);
            }
            owner.close().await?;

            let mut successor = ServiceOwner::open(options, &project).await?;
            let current = successor.authority().clone();
            let current_record =
                EndpointRecord::read(&data, &scope)?.context("missing successor endpoint")?;
            for (query_id, candidate_proof, expected) in [
                (id, proof, rpc::OutcomeStatus::Committed),
                (Uuid::new_v4(), missing, rpc::OutcomeStatus::Absent),
            ] {
                let mut client =
                    connect_local(&data, &scope, &current_record.address, HANDSHAKE_TIMEOUT)
                        .await?;
                let mut server = successor.accept(HANDSHAKE_TIMEOUT).await?;
                let (response, served) = tokio::join!(
                    rpc::request_one(&mut client, &current, query(query_id, candidate_proof)),
                    rpc::serve_one(&mut server, &current, &successor.store),
                );
                ensure!(matches!(response?, ServiceValue::Outcome(status) if status == expected));
                served?;
                drop(server);
                drop(client);
            }
            let ledger = successor.store.usage_ledger()?;
            ledger.admit(start.clone()).await?;
            ensure!(
                ledger.session(&start.session_id).await?.invocation_count == 2,
                "exact usage retry charged the original invocation twice"
            );
            drop(ledger);
            successor.close().await
        })
        .await
        .context("lost usage reply fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_transition_query_preserves_open_conflict_and_promoted_revision() -> Result<()>
    {
        use crate::service::rpc::{CandidateTransitionKind, CandidateTransitionResult};

        async fn inspect(
            owner: &mut ServiceOwner,
            authority: &EndpointAuthority,
            data: &Path,
            scope: &str,
            generation: &str,
            transition: CandidateTransitionKind,
            subject: (&str, &str, &str),
        ) -> Result<CandidateTransitionResult> {
            let (branch, base, target) = subject;
            let record = EndpointRecord::read(data, scope)?.context("missing endpoint")?;
            let mut client = connect_local(data, scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut server = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let (response, served) = tokio::join!(
                rpc::request_one(
                    &mut client,
                    authority,
                    ServiceCall::CandidateTransitionOutcome {
                        original_id: uuid::Uuid::new_v4(),
                        original_generation: generation.to_owned(),
                        transition,
                        branch: branch.to_owned(),
                        base: base.to_owned(),
                        target: target.to_owned(),
                    },
                ),
                rpc::serve_one(&mut server, authority, &owner.store),
            );
            served?;
            let ServiceValue::CandidateTransitionOutcome(result) = response? else {
                bail!("candidate transition query returned the wrong typed response")
            };
            Ok(result)
        }

        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!("project/{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
            let data = root.path().join("private");
            let options = crate::test_support::open_options(data.clone(), scope.clone())?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            let original = owner.authority().clone();
            let candidate = owner.store.begin_candidate("transition outcome").await?;
            let branch = candidate.view().pinned_view().to_owned();
            let base = candidate.base().to_owned();
            candidate.view().put("private", &serde_json::json!(1)).await?;
            let target = candidate.view().revision().await?;
            ensure!(matches!(
                inspect(&mut owner, &original, &data, &scope, &original.service_generation, CandidateTransitionKind::Promote, (&branch, &base, &target)).await?,
                CandidateTransitionResult::StillUncertain
            ));
            owner.store.put("sibling", &serde_json::json!(true)).await?;
            ensure!(matches!(
                inspect(&mut owner, &original, &data, &scope, &original.service_generation, CandidateTransitionKind::Promote, (&branch, &base, &target)).await?,
                CandidateTransitionResult::StillUncertain
            ));
            drop(candidate);
            owner.close().await?;

            let mut successor = ServiceOwner::open(options, &project).await?;
            let current = successor.authority().clone();
            ensure!(matches!(
                inspect(&mut successor, &current, &data, &scope, &original.service_generation, CandidateTransitionKind::Promote, (&branch, &base, &target)).await?,
                CandidateTransitionResult::OpenConflict
            ));
            let new_candidate = successor.store.begin_candidate("later promoted").await?;
            let new_branch = new_candidate.view().pinned_view().to_owned();
            let new_base = new_candidate.base().to_owned();
            new_candidate.view().put("promoted", &serde_json::json!(2)).await?;
            let new_target = new_candidate.view().revision().await?;
            ensure!(new_candidate.promote_exact(&new_target).await? == new_target);
            successor.store.put("after-promotion", &serde_json::json!(3)).await?;
            ensure!(matches!(
                inspect(&mut successor, &current, &data, &scope, &original.service_generation, CandidateTransitionKind::Promote, (&new_branch, &new_base, &new_target)).await?,
                CandidateTransitionResult::Promoted { revision } if revision == new_target
            ));
            ensure!(matches!(
                inspect(&mut successor, &current, &data, &scope, &original.service_generation, CandidateTransitionKind::Abandon, (&new_branch, &new_base, &new_target)).await?,
                CandidateTransitionResult::PreservedConflict
            ));
            drop(new_candidate);
            successor.close().await
        })
        .await
        .context("candidate transition outcome fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_candidate_transition_replies_survive_sibling_write_and_owner_restart()
    -> Result<()> {
        use crate::service::rpc::{
            CandidateTransitionKind, CandidateTransitionResult, ViewOperation,
        };

        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!("project/{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
            let options = crate::test_support::open_options(root.path().join("private"), scope)?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let served = tokio::spawn(owner.serve());
            let mut client = attach_existing(&options, &project).await?.context("missing service attachment")?;
            let generation = client.generation().to_owned();
            let ServiceValue::CandidateStarted { handle, base, branch } = client
                .call(ServiceCall::BeginCandidate { label: "lost promotion".into() })
                .await? else { bail!("service did not start the candidate") };
            ensure!(matches!(client.call(ServiceCall::View {
                candidate: Some(handle),
                operation: ViewOperation::PutMany { values: vec![("private".into(), serde_json::json!(1))] },
            }).await?, ServiceValue::Unit));
            let ServiceValue::Revision(target) = client.call(ServiceCall::View {
                candidate: Some(handle), operation: ViewOperation::Revision,
            }).await? else { bail!("service did not report candidate target") };
            let id = uuid::Uuid::new_v4();
            let request = rpc::ServiceRequest::with_id(&generation, id,
                ServiceCall::PromoteCandidate {
                    handle, branch: branch.clone(), base: base.clone(), target: target.clone(),
                });
            let body = serde_json::to_vec(&request)?;
            let mut stream = client.stream.take().context("missing live candidate stream")?;
            tokio::time::timeout(Duration::from_secs(5), async {
                stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
                stream.write_all(&body).await?;
                stream.flush().await
            }).await.context("candidate promotion frame send deadline")??;
            drop(stream); // the accepted request may complete; its reply is lost
            drop(client);

            let query = || ServiceCall::CandidateTransitionOutcome {
                original_id: id, original_generation: generation.clone(),
                transition: CandidateTransitionKind::Promote,
                branch: branch.clone(), base: base.clone(), target: target.clone(),
            };
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let mut observer = attach_existing(&options, &project).await?
                        .context("owner disappeared before promotion proof")?;
                    match observer.call(query()).await? {
                        ServiceValue::CandidateTransitionOutcome(CandidateTransitionResult::Promoted { revision })
                            if revision == target => break Ok::<(), anyhow::Error>(()),
                        ServiceValue::CandidateTransitionOutcome(
                            CandidateTransitionResult::InFlight | CandidateTransitionResult::StillUncertain,
                        ) => tokio::time::sleep(Duration::from_millis(20)).await,
                        other => bail!("candidate promotion returned unexpected outcome: {other:?}"),
                    }
                }
            }).await.context("accepted promotion proof deadline")??;
            let mut sibling = attach_existing(&options, &project).await?
                .context("missing service for sibling write")?;
            ensure!(matches!(sibling.call(ServiceCall::PutMany {
                values: vec![("later-main".into(), serde_json::json!(true))],
            }).await?, ServiceValue::Unit));
            drop(sibling);
            let mut observer = attach_existing(&options, &project).await?
                .context("missing service after sibling write")?;
            ensure!(matches!(observer.call(query()).await?,
                ServiceValue::CandidateTransitionOutcome(CandidateTransitionResult::Promoted { revision })
                    if revision == target));
            drop(observer);
            let mut abandoner = attach_existing(&options, &project).await?
                .context("missing service for accepted abandonment")?;
            let abandon_generation = abandoner.generation().to_owned();
            let ServiceValue::CandidateStarted {
                handle: abandon_handle,
                base: abandon_base,
                branch: abandon_branch,
            } = abandoner.call(ServiceCall::BeginCandidate {
                label: "lost abandonment".into(),
            }).await? else { bail!("service did not start the abandonment candidate") };
            ensure!(matches!(abandoner.call(ServiceCall::View {
                candidate: Some(abandon_handle),
                operation: ViewOperation::PutMany {
                    values: vec![("abandoned-private".into(), serde_json::json!(1))],
                },
            }).await?, ServiceValue::Unit));
            let ServiceValue::Revision(abandon_target) = abandoner.call(ServiceCall::View {
                candidate: Some(abandon_handle), operation: ViewOperation::Revision,
            }).await? else { bail!("service did not report the abandonment target") };
            let abandon_id = uuid::Uuid::new_v4();
            let request = rpc::ServiceRequest::with_id(&abandon_generation, abandon_id,
                ServiceCall::AbandonCandidate {
                    handle: abandon_handle,
                    branch: abandon_branch.clone(),
                    base: abandon_base.clone(),
                    target: abandon_target.clone(),
                });
            let body = serde_json::to_vec(&request)?;
            let mut stream = abandoner.stream.take().context("missing abandonment stream")?;
            tokio::time::timeout(Duration::from_secs(5), async {
                stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
                stream.write_all(&body).await?;
                stream.flush().await
            }).await.context("candidate abandonment frame send deadline")??;
            drop(stream);
            drop(abandoner);
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let mut observer = attach_existing(&options, &project).await?
                        .context("owner disappeared before abandonment proof")?;
                    match observer.call(ServiceCall::CandidateTransitionOutcome {
                        original_id: abandon_id,
                        original_generation: abandon_generation.clone(),
                        transition: CandidateTransitionKind::Abandon,
                        branch: abandon_branch.clone(),
                        base: abandon_base.clone(),
                        target: abandon_target.clone(),
                    }).await? {
                        ServiceValue::CandidateTransitionOutcome(CandidateTransitionResult::Abandoned) =>
                            break Ok::<(), anyhow::Error>(()),
                        ServiceValue::CandidateTransitionOutcome(
                            CandidateTransitionResult::InFlight | CandidateTransitionResult::StillUncertain,
                        ) => tokio::time::sleep(Duration::from_millis(20)).await,
                        other => bail!("candidate abandonment returned unexpected outcome: {other:?}"),
                    }
                }
            }).await.context("accepted abandonment proof deadline")??;
            let permit = acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await.context("promotion fixture owner did not reap")???;
            drop(permit);
            let successor = ServiceOwner::open(options.clone(), &project).await?;
            let successor_served = tokio::spawn(successor.serve());
            let mut observer = attach_existing(&options, &project).await?
                .context("successor did not publish its endpoint")?;
            ensure!(observer.generation() != generation);
            ensure!(matches!(observer.call(query()).await?,
                ServiceValue::CandidateTransitionOutcome(CandidateTransitionResult::Promoted { revision })
                    if revision == target));
            ensure!(matches!(observer.call(ServiceCall::CandidateTransitionOutcome {
                original_id: abandon_id,
                original_generation: abandon_generation,
                transition: CandidateTransitionKind::Abandon,
                branch: abandon_branch,
                base: abandon_base,
                target: abandon_target,
            }).await?, ServiceValue::CandidateTransitionOutcome(CandidateTransitionResult::Abandoned)));
            drop(observer);
            let permit = acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), successor_served)
                .await.context("transition fixture successor did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        }).await.context("lost candidate promotion fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_candidate_begin_reply_uses_only_read_only_exact_ref_outcomes() -> Result<()> {
        use crate::store::CandidateLookup;
        use rpc::CandidateCreationOutcome;
        use tokio::io::AsyncWriteExt;
        use uuid::Uuid;

        async fn inspect(
            owner: &mut ServiceOwner,
            authority: &EndpointAuthority,
            data: &Path,
            scope: &str,
            id: Uuid,
            generation: &str,
        ) -> Result<CandidateCreationOutcome> {
            let record = EndpointRecord::read(data, scope)?.context("missing endpoint")?;
            let mut client = connect_local(data, scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut server = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let (response, served) = tokio::join!(
                rpc::request_one(
                    &mut client,
                    authority,
                    ServiceCall::CandidateOutcome {
                        original_id: id,
                        original_generation: generation.to_owned(),
                    },
                ),
                rpc::serve_one(&mut server, authority, &owner.store),
            );
            served?;
            let ServiceValue::CandidateOutcome(outcome) = response? else {
                bail!("candidate query returned the wrong typed response")
            };
            Ok(outcome)
        }

        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let options = crate::test_support::open_options(data.clone(), scope.clone())?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            let authority = owner.authority().clone();
            let original_generation = authority.service_generation.clone();
            let id = Uuid::new_v4();
            let record = EndpointRecord::read(&data, &scope)?.context("missing endpoint")?;
            let request = rpc::ServiceRequest::with_id(
                &original_generation,
                id,
                ServiceCall::BeginCandidate {
                    label: "lost-begin".into(),
                },
            );
            let body = serde_json::to_vec(&request)?;
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let mut server = owner.accept(HANDSHAKE_TIMEOUT).await?;
            let sent = async {
                connect_handshake(&mut client, &authority).await?;
                client.write_all(&(body.len() as u32).to_be_bytes()).await?;
                client.write_all(&body).await?;
                client.flush().await?;
                drop(client); // the caller has no candidate handle or base
                Ok::<(), anyhow::Error>(())
            };
            let (sent, _) =
                tokio::join!(sent, rpc::serve_one(&mut server, &authority, &owner.store));
            sent?;
            drop(server);

            let (branch, base) = match inspect(
                &mut owner,
                &authority,
                &data,
                &scope,
                id,
                &original_generation,
            )
            .await?
            {
                CandidateCreationOutcome::Open { branch, base, .. } => (branch, base),
                _ => bail!("lost Begin did not retain its exact candidate ref"),
            };
            for _ in 0..2 {
                let CandidateCreationOutcome::Open {
                    branch: observed,
                    base: observed_base,
                    ..
                } = inspect(
                    &mut owner,
                    &authority,
                    &data,
                    &scope,
                    id,
                    &original_generation,
                )
                .await?
                else {
                    bail!("repeat candidate outcome lost the open ref")
                };
                ensure!(observed == branch && observed_base == base);
            }
            let CandidateLookup::Open(retained) = owner.store.candidate_for_id(id).await? else {
                bail!("accepted Begin lost its exact candidate before restart")
            };
            retained
                .view()
                .put("private/lost-begin", &serde_json::json!("retained"))
                .await?;
            let private_head = retained.view().revision().await?;
            drop(retained);
            owner
                .store
                .put("later/main", &serde_json::json!("sibling"))
                .await?;
            owner.close().await?;

            let mut successor = ServiceOwner::open(options.clone(), &project).await?;
            let next = successor.authority().clone();
            ensure!(next.service_generation != original_generation);
            let CandidateCreationOutcome::Open {
                branch: observed,
                base: observed_base,
                ..
            } = inspect(
                &mut successor,
                &next,
                &data,
                &scope,
                id,
                &original_generation,
            )
            .await?
            else {
                bail!("successor did not reattach the exact candidate ref")
            };
            ensure!(observed == branch && observed_base == base);
            let CandidateLookup::Open(retained) = successor.store.candidate_for_id(id).await?
            else {
                bail!("successor did not retain the exact candidate contents")
            };
            ensure!(
                retained.view().revision().await? == private_head,
                "candidate head changed while recovering the lost Begin reply"
            );
            ensure!(
                retained.view().get("private/lost-begin").await?
                    == Some(serde_json::json!("retained")),
                "candidate private rows disappeared across owner restart"
            );
            drop(retained);
            successor.close().await?;

            let local = crate::store::MemoryStore::open(options.clone()).await?;
            let CandidateLookup::Open(candidate) = local.candidate_for_id(id).await? else {
                bail!("exact candidate was missing before explicit abandonment")
            };
            candidate.abandon().await?;
            drop(candidate);
            local.close().await?;
            let mut final_owner = ServiceOwner::open(options, &project).await?;
            let final_authority = final_owner.authority().clone();
            ensure!(matches!(
                inspect(
                    &mut final_owner,
                    &final_authority,
                    &data,
                    &scope,
                    id,
                    &original_generation,
                )
                .await?,
                CandidateCreationOutcome::StillUncertain
            ));
            ensure!(matches!(
                final_owner.store.candidate_for_id(id).await?,
                CandidateLookup::Missing
            ));
            final_owner.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("candidate lost-reply outcome fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn independent_clients_elect_one_real_process_and_keep_it_warm() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(120), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            let executable = crate::store::test_supervisor()?;
            options.supervisor = Some(executable.clone());
            let _gate = crate::spawn_gate::spawning().await;
            let (first, second) = tokio::join!(
                attach_or_start(&options, &project, &executable),
                attach_or_start(&options, &project, &executable),
            );
            let mut first = first?;
            let mut second = second?;
            ensure!(
                first.generation() == second.generation(),
                "simultaneous starters attached different service generations"
            );
            let generation = first.generation().to_owned();
            let (one, two) = tokio::join!(
                first.call(ServiceCall::AppendMessage {
                    namespace: "election-fixture".into(),
                    message: kuru_core::Message::text("user", "one"),
                }),
                second.call(ServiceCall::AppendMessage {
                    namespace: "election-fixture".into(),
                    message: kuru_core::Message::text("user", "two"),
                }),
            );
            ensure!(matches!(one?, ServiceValue::Unit));
            ensure!(matches!(two?, ServiceValue::Unit));
            drop(first);
            // A surviving attachment keeps the same engine available after
            // its starter-side peer exits and can read both committed rows.
            let ServiceValue::HistoryWindow(window) = second
                .call(ServiceCall::HistoryWindow {
                    namespace: "election-fixture".into(),
                    limit: 4,
                })
                .await?
            else {
                bail!("history request returned the wrong result type");
            };
            ensure!(window.total_rows == 2 && window.messages.len() == 2);
            ensure!(
                EndpointRecord::read(&data, &scope)?
                    .is_some_and(|record| record.authority.service_generation == generation),
                "live attachment lost its service owner"
            );
            drop(second);
            let deadline =
                tokio::time::Instant::now() + SERVICE_IDLE_TIMEOUT + Duration::from_secs(20);
            loop {
                if EndpointRecord::read(&data, &scope)?.is_none() {
                    break;
                }
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "idle service did not retire its endpoint after reaping Dolt"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // Endpoint retirement happens before the service owner drops its
            // retained lock. Observe both steps, without mistaking that
            // brief ordering interval for a leaked owner.
            while ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_none() {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "idle service did not release owner authority after endpoint retirement"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("multiprocess memory service fixture exceeded 120 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn rejected_publication_reaps_engine_before_owner_lock_releases() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(crate::store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            #[cfg(unix)]
            let obstruction = {
                use std::os::unix::fs::{MetadataExt, symlink};
                let uid = std::fs::metadata(root.path())?.uid();
                let parent = PathBuf::from(format!("/tmp/kuru-service-{uid}"));
                Directory::ensure_private(&parent)?;
                let path = parent.join(unix_socket_locator(&scope)?);
                symlink("missing-native-socket-directory", &path)?;
                path
            };
            #[cfg(windows)]
            let obstruction = {
                let path = EndpointRecord::directory(&data, &scope)?;
                Directory::ensure_private(
                    path.parent().context("service directory has no parent")?,
                )?;
                std::fs::write(&path, b"not a directory")?;
                path
            };
            ensure!(
                ServiceOwner::open(options.clone(), &project).await.is_err(),
                "blocked native publication unexpectedly published an owner"
            );
            std::fs::remove_file(obstruction)?;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "failed publication left a discoverable endpoint"
            );
            let reopened = ServiceOwner::open(options, &project).await?;
            reopened.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("publication cleanup fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn idle_accept_deadlines_do_not_close_live_attachment() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(crate::store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options, &project).await?;
            let record =
                EndpointRecord::read(&data, &scope)?.context("missing service endpoint")?;
            let mut client =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            let served = tokio::spawn(async move {
                owner
                    .serve_until_idle_with(Duration::from_millis(50), Duration::from_millis(100))
                    .await?;
                owner.close().await
            });
            connect_handshake(&mut client, &record.authority).await?;
            tokio::time::sleep(Duration::from_millis(350)).await;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_some(),
                "live attachment was retired after an accept timeout"
            );
            ensure!(
                matches!(
                    rpc::request_attached(&mut client, &record.authority, ServiceCall::Revision)
                        .await?,
                    ServiceValue::Revision(_)
                ),
                "surviving attachment could not read after accept deadlines"
            );
            drop(client);
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("owner did not close after the fixture idle interval")???;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("live attachment accept-timeout fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn separate_cold_starters_share_one_owner_and_preserve_both_writes() -> Result<()> {
        use std::process::Stdio;
        tokio::time::timeout(Duration::from_secs(100), async {
            let root = tempfile::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
            options.config.cache_dir = Some(crate::store::test_cache());
            options.config.offline = true;
            let executable = crate::store::test_supervisor()?;
            options.supervisor = Some(executable.clone());
            let barrier = root.path().join("go");
            let ready_one = root.path().join("ready-one");
            let ready_two = root.path().join("ready-two");
            let _gate = crate::spawn_gate::spawning().await;
            let spawn = |ready: &Path| -> Result<tokio::process::Child> {
                let mut args = service_arguments(&options, &project);
                args[0] = "--internal-memory-service-client-fixture".into();
                args.push(barrier.as_os_str().to_owned());
                args.push(ready.as_os_str().to_owned());
                let child = tokio::process::Command::new(&executable)
                    .args(args)
                    .current_dir(&project)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()?;
                Ok(child)
            };
            let one = spawn(&ready_one)?;
            let two = spawn(&ready_two)?;
            let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while !ready_one.exists() || !ready_two.exists() {
                ensure!(
                    tokio::time::Instant::now() < ready_deadline,
                    "client fixture processes did not reach their start barrier"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            std::fs::write(&barrier, b"go")?;
            let (one, two) = tokio::join!(
                tokio::time::timeout(Duration::from_secs(60), one.wait_with_output()),
                tokio::time::timeout(Duration::from_secs(60), two.wait_with_output()),
            );
            let one = one.context("first cold starter deadline exceeded")??;
            let two = two.context("second cold starter deadline exceeded")??;
            ensure!(
                one.status.success(),
                "first cold starter failed: {}",
                String::from_utf8_lossy(&one.stderr)
            );
            ensure!(
                two.status.success(),
                "second cold starter failed: {}",
                String::from_utf8_lossy(&two.stderr)
            );
            let generation_one = String::from_utf8(one.stdout)?.trim().to_owned();
            let generation_two = String::from_utf8(two.stdout)?.trim().to_owned();
            ensure!(
                !generation_one.is_empty() && generation_one == generation_two,
                "cold starters reached different service generations"
            );
            let mut client = attach_or_start(&options, &project, &executable).await?;
            ensure!(
                client.generation() == generation_one,
                "cold service did not survive its starter processes"
            );
            let ServiceValue::HistoryWindow(window) = client
                .call(ServiceCall::HistoryWindow {
                    namespace: "multiprocess-fixture".into(),
                    limit: 4,
                })
                .await?
            else {
                bail!("multiprocess history returned wrong type");
            };
            ensure!(
                window.total_rows == 2 && window.messages.len() == 2,
                "cold starters lost a committed row"
            );
            drop(client);
            let idle_deadline =
                tokio::time::Instant::now() + SERVICE_IDLE_TIMEOUT + Duration::from_secs(20);
            loop {
                if EndpointRecord::read(&data, &scope)?.is_none() {
                    break;
                }
                ensure!(
                    tokio::time::Instant::now() < idle_deadline,
                    "cold-start fixture service did not reap before its directory cleanup"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            while ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_none() {
                ensure!(
                    tokio::time::Instant::now() < idle_deadline,
                    "cold-start fixture service did not release owner authority after endpoint retirement"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("separate cold starter fixture exceeded 100 seconds")??;
        Ok(())
    }
}
