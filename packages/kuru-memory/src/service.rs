//! Private local protocol for the project memory owner.
//!
//! This module contains no provider, tool or conversation control operation.
//! Transport privacy excludes other OS users; a same-user process able to read
//! the private endpoint record is within the account's local authority.

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

mod rpc;
pub use rpc::{ServiceCall, ServiceReply, ServiceRequest, ServiceResponse, ServiceValue};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 0;
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
}

impl ServiceAttachment {
    pub fn generation(&self) -> &str {
        &self.authority.service_generation
    }

    pub async fn call(&mut self, call: ServiceCall) -> Result<ServiceValue> {
        let mut stream = self
            .stream
            .take()
            .context("memory service attachment is closed")?;
        let result = rpc::request_attached(&mut stream, &self.authority, call).await;
        if result.is_ok() {
            self.stream = Some(stream);
        }
        result
    }

    pub fn close(&mut self) {
        self.stream = None;
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
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project).await? {
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
        if let Some(attached) =
            try_attach(&options.data_dir, &options.project_scope, project).await?
        {
            return Ok(attached);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project).await? {
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
        if let Some(attached) =
            try_attach(&options.data_dir, &options.project_scope, project).await?
        {
            return Ok(attached);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut child = ServiceProcess::new(spawn_service(options, project, executable).await?);
    loop {
        if let Some(attached) =
            try_attach(&options.data_dir, &options.project_scope, project).await?
        {
            return Ok(attached);
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
) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(executable);
    command.args(service_arguments(options, project));
    command.current_dir(project);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    command.process_group(0);
    command.spawn().context("start project memory service")
}

#[cfg(windows)]
async fn spawn_service(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
) -> Result<kuru_platform::windows::process::NativeChild> {
    use kuru_platform::windows::process::{Console, Lifetime, NativeSpawnSpec};
    let mut command = NativeSpawnSpec::new(executable.to_owned(), project.to_owned());
    command.args = service_arguments(options, project);
    command.lifetime = Lifetime::IndependentService;
    command.console = Console::PrivateHidden;
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
        loop {
            if let Err(error) = self.lock.verify() {
                abort_and_drain(&mut attachments).await;
                return Err(error);
            }
            if attachments.is_empty() {
                tokio::select! {
                    biased;
                    accepted = self.listener.accept(accept_timeout) => {
                        self.attach(accepted?, &mut attachments, &frame_budget);
                    }
                    _ = tokio::time::sleep(idle_timeout) => break,
                }
            } else {
                tokio::select! {
                    accepted = self.listener.accept(accept_timeout) => {
                        match accepted {
                            Ok(stream) => self.attach(stream, &mut attachments, &frame_budget),
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
    ) {
        if attachments.len() >= MAX_ATTACHMENTS {
            return;
        }
        let authority = self.record.authority.clone();
        let store = self.store.clone();
        let frame_budget = frame_budget.clone();
        attachments.spawn(async move {
            let mut stream = stream;
            if let Err(error) =
                rpc::serve_attached(&mut stream, &authority, &store, frame_budget).await
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
            NameRetention::Pinned,
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
    use tokio::io::duplex;

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
            ensure!(
                ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_some(),
                "idle service retained owner authority after endpoint retirement"
            );
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
            ensure!(
                ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?.is_some(),
                "cold-start fixture service retained owner authority after idle cleanup"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("separate cold starter fixture exceeded 100 seconds")??;
        Ok(())
    }
}
