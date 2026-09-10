//! A private Dolt sidecar owned by a lifetime-pipe supervisor.
//!
//! Numeric PIDs are never recovery authority. The supervisor retains its child
//! until it is reaped and holds the stable lifecycle lock through that point.

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{engine::Child, files};
use anyhow::{Context, Result, anyhow, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
#[cfg(windows)]
use kuru_platform::windows::{
    pipe,
    process::{Console, Lifetime, NativeChild as SupervisorChild, NativeSpawnSpec},
};
#[cfg(unix)]
use std::{
    os::{
        fd::{AsFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child as SupervisorChild, Command, Stdio},
};
#[cfg(unix)]
use tokio::net::unix::pipe;
#[cfg(unix)]
type LifetimeSender = pipe::Sender;
#[cfg(windows)]
type LifetimeSender = pipe::Pipe;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{
    MySqlPool, Row,
    mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::{Instant, sleep, timeout},
};
use uuid::Uuid;

const RECORD_LIMIT: usize = 64 * 1024;
const LOG_LIMIT: usize = 32 * 1024;
const CLOSE_GRACE: Duration = Duration::from_secs(8);
const KILL_GRACE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub binary: PathBuf,
    pub directory: PathBuf,
    pub project_scope: String,
    pub supervisor: PathBuf,
    pub timeout: Duration,
    pub read_only: bool,
    pub retained: Option<Arc<tempfile::TempDir>>,
    /// Windows requires one stable external namespace for all names of this
    /// physical store. Unix preserves its existing in-directory lease layout.
    pub lifecycle_root: Option<PathBuf>,
}

#[derive(Clone)]
pub struct Server(Arc<ServerInner>);

struct ServerInner {
    directory: PathBuf,
    identity: Identity,
    endpoint: Endpoint,
    read_only: bool,
    pools: Mutex<BTreeMap<String, Weak<MySqlPool>>>,
    owner: Mutex<Option<Owner>>,
    closed: AtomicBool,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("directory", &self.0.directory)
            .field("instance", &self.0.identity.instance)
            .field("read_only", &self.0.read_only)
            .field("closed", &self.0.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

// Dropping this closes stdin. Do not kill the supervisor on drop: it needs to
// receive EOF and reap Dolt, including when an open/close future is cancelled.
struct Owner {
    child: Option<SupervisorChild>,
    lifetime: Option<LifetimeSender>,
    retained: Option<Arc<tempfile::TempDir>>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        drop(self.lifetime.take());
        let Some(child) = self.child.take() else {
            return;
        };
        let retained = self.retained.take();
        // Independent of Tokio: tests and CLI shutdown may destroy the runtime
        // immediately after the last store handle. Keep fixture files until the
        // supervisor has confirmed that Dolt is reaped.
        std::thread::spawn(move || {
            observe_supervisor(
                child,
                retained,
                CLOSE_GRACE + KILL_GRACE + Duration::from_secs(3),
                SupervisorChild::try_wait,
            )
        });
    }
}

fn observe_supervisor(
    mut child: SupervisorChild,
    retained: Option<Arc<tempfile::TempDir>>,
    warn_after: Duration,
    mut status: impl FnMut(&mut SupervisorChild) -> std::io::Result<Option<std::process::ExitStatus>>,
) {
    let deadline = std::time::Instant::now() + warn_after;
    let mut warned = false;
    loop {
        let result = status(&mut child);
        if matches!(result, Ok(Some(_))) {
            return;
        }
        if !warned && (result.is_err() || std::time::Instant::now() >= deadline) {
            eprintln!(
                "memory supervisor cleanup is delayed; retaining its process handle and resources{}",
                retained
                    .as_ref()
                    .map_or_else(String::new, |directory| format!(
                        " at {}",
                        directory.path().display()
                    ))
            );
            warned = true;
        }
        // This is continued observation of the same retained process. A close
        // deadline or failed status query grants no permission to delete data.
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: u32,
    instance: String,
    project_scope: String,
    password: String,
    reader_password: String,
    initialized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    instance: String,
    port: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    binary: PathBuf,
    directory: PathBuf,
    project_scope: String,
    timeout_millis: u64,
    read_only: bool,
    lifecycle_root: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Response {
    Ready { endpoint: Endpoint, owned: bool },
    Failed(String),
}

impl Server {
    /// Hold the same stable lease as the supervisor until the caller completes
    /// a stopped-store rename and directory fsync. Closing an attached handle is
    /// not proof of quiescence: only acquiring this lease establishes it.
    #[cfg(all(test, unix))]
    pub(crate) async fn quiescence(directory: &Path, wait: Duration) -> Result<LifecycleLease> {
        Self::quiescence_at(directory, None, wait).await
    }

    pub(crate) async fn quiescence_at(
        directory: &Path,
        lifecycle_root: Option<&Path>,
        wait: Duration,
    ) -> Result<LifecycleLease> {
        ensure!(
            !wait.is_zero() && wait <= Duration::from_secs(300),
            "invalid memory quiescence timeout"
        );
        prepare_directory(directory, true)?;
        let directory = fs::canonicalize(directory)?;
        let lease = LifecycleLease::new(&directory, lifecycle_root)?;
        let deadline = Instant::now() + wait;
        loop {
            lease.verify()?;
            match lease.lock.try_lock() {
                Ok(()) => {
                    lease.verify()?;
                    return Ok(lease);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    ensure!(
                        Instant::now() < deadline,
                        "memory lifecycle remains active; refusing to move its directory"
                    );
                    sleep(
                        Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    pub async fn open(options: ServerOptions) -> Result<Self> {
        ensure!(
            options.timeout >= Duration::from_millis(1)
                && options.timeout <= Duration::from_secs(300),
            "invalid memory server startup timeout"
        );
        ensure!(
            !options.project_scope.is_empty() && options.project_scope.len() <= 4096,
            "invalid memory project scope"
        );
        prepare_directory(&options.directory, options.read_only)?;
        let directory = fs::canonicalize(&options.directory)?;
        LifecycleLease::validate_root(&directory, options.lifecycle_root.as_deref())?;
        if let Some(identity) = load_identity(&directory, &options.project_scope)? {
            // Validate a published endpoint even for a writer, but only readers
            // may borrow another parent's lifetime. A writer must wait for the
            // stable lease and retain its own supervisor before exposing pools.
            if let Some(endpoint) = live_endpoint(&directory, &identity, options.read_only).await?
                && options.read_only
            {
                return Ok(Self::new(
                    directory,
                    identity,
                    endpoint,
                    options.read_only,
                    None,
                ));
            }
        } else {
            ensure!(!options.read_only, "memory has not been initialized");
        }

        let request = Request {
            binary: options.binary.clone(),
            directory: directory.clone(),
            project_scope: options.project_scope.clone(),
            timeout_millis: options.timeout.as_millis().try_into()?,
            read_only: options.read_only,
            lifecycle_root: options.lifecycle_root.clone(),
        };
        #[cfg(unix)]
        let (mut owner, response) = {
            let mut command = Command::new(options.supervisor);
            command
                .arg("--internal-dolt-supervisor")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .current_dir(&directory)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0);
            // The supervisor is this Rust program, so instrumented test binaries
            // must retain their designated profile output through environment
            // isolation. Never pass this or parent credentials to the Dolt engine.
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            let mut child = command
                .spawn()
                .context("start memory lifetime supervisor")?;
            let lifetime = child
                .stdin
                .take()
                .context("supervisor lifetime pipe missing")?;
            let output = child
                .stdout
                .take()
                .context("supervisor readiness pipe missing")?;
            let mut owner = Owner {
                child: Some(child),
                lifetime: None,
                retained: options.retained.clone(),
            };
            owner.lifetime = Some(pipe::Sender::from_owned_fd(OwnedFd::from(lifetime))?);
            let mut output = pipe::Receiver::from_owned_fd(OwnedFd::from(output))?;
            let response = timeout(options.timeout + Duration::from_secs(2), async {
                write_frame(owner.lifetime.as_mut().expect("owned lifetime"), &request).await?;
                read_frame::<_, Response>(&mut output).await
            })
            .await
            .context("memory supervisor readiness deadline exceeded")
            .and_then(|response| response);
            (owner, response)
        };
        #[cfg(windows)]
        let (mut owner, response) = {
            let listener = pipe::PrivateListener::bind()?;
            let mut command = NativeSpawnSpec::new(options.supervisor.clone(), directory.clone());
            command.args = vec![
                "--internal-dolt-supervisor".into(),
                listener.address().to_owned(),
            ];
            command.lifetime = Lifetime::TrustedSupervisor;
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
            let child = command
                .spawn()
                .await
                .context("start memory lifetime supervisor")?;
            let accept = listener.accept(&child, Duration::from_secs(5));
            let mut owner = Owner {
                child: Some(child),
                lifetime: None,
                retained: options.retained.clone(),
            };
            let response = timeout(options.timeout + Duration::from_secs(2), async {
                owner.lifetime = Some(accept.await?);
                let channel = owner.lifetime.as_mut().expect("owned lifetime");
                write_frame(channel, &request).await?;
                read_frame::<_, Response>(channel).await
            })
            .await
            .context("memory supervisor readiness deadline exceeded")
            .and_then(|response| response);
            (owner, response)
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let _ = finish_owner(&mut owner).await;
                return Err(error);
            }
        };
        let (endpoint, owned) = match response {
            Response::Ready { endpoint, owned } => {
                if !owned && !options.read_only {
                    let _ = finish_owner(&mut owner).await;
                    bail!("writable memory requires an owned supervisor lifetime");
                }
                (endpoint, owned)
            }
            Response::Failed(message) => {
                let _ = finish_owner(&mut owner).await;
                bail!("memory server startup failed: {message}");
            }
        };
        let verified = async {
            let identity = load_identity(&directory, &options.project_scope)?
                .context("memory identity missing after startup")?;
            let probe = connect_pool(
                &identity,
                &endpoint,
                &directory,
                "main",
                options.read_only,
                1,
            )
            .await?;
            let result = verify_identity(&probe, &directory, &identity).await;
            probe.close().await;
            result?;
            Ok::<_, anyhow::Error>(identity)
        }
        .await;
        let identity = match verified {
            Ok(identity) => identity,
            Err(error) => {
                let _ = finish_owner(&mut owner).await;
                return Err(error);
            }
        };
        let owner = if owned {
            Some(owner)
        } else {
            finish_owner(&mut owner).await?;
            None
        };
        Ok(Self::new(
            directory,
            identity,
            endpoint,
            options.read_only,
            owner,
        ))
    }

    fn new(
        directory: PathBuf,
        identity: Identity,
        endpoint: Endpoint,
        read_only: bool,
        owner: Option<Owner>,
    ) -> Self {
        Self(Arc::new(ServerInner {
            directory,
            identity,
            endpoint,
            read_only,
            pools: Mutex::new(BTreeMap::new()),
            owner: Mutex::new(owner),
            closed: AtomicBool::new(false),
        }))
    }

    pub async fn pool(&self, branch: &str) -> Result<Arc<MySqlPool>> {
        validate_branch(branch)?;
        let mut pools = self.0.pools.lock().await;
        ensure!(
            !self.0.closed.load(Ordering::Acquire),
            "memory server is closed"
        );
        if let Some(pool) = pools.get(branch).and_then(Weak::upgrade) {
            return Ok(pool);
        }
        pools.retain(|_, pool| pool.strong_count() != 0);
        let pool = connect_pool(
            &self.0.identity,
            &self.0.endpoint,
            &self.0.directory,
            branch,
            self.0.read_only,
            4,
        )
        .await?;
        verify_identity(&pool, &self.0.directory, &self.0.identity).await?;
        let pool = Arc::new(pool);
        pools.insert(branch.to_owned(), Arc::downgrade(&pool));
        Ok(pool)
    }

    pub async fn close(&self) -> Result<()> {
        // The owner mutex also makes concurrent close callers wait for reaping.
        let mut owner = self.0.owner.lock().await;
        self.0.closed.store(true, Ordering::Release);
        let pools = std::mem::take(&mut *self.0.pools.lock().await);
        let pool_result = timeout(CLOSE_GRACE, async {
            for pool in pools.values().filter_map(Weak::upgrade) {
                pool.close().await;
            }
        })
        .await;
        let stop_result = if let Some(mut owned) = owner.take() {
            finish_owner(&mut owned).await
        } else {
            Ok(())
        };
        stop_result?;
        pool_result.context("memory pool close deadline exceeded")?;
        Ok(())
    }

    /// Transfer an ephemeral fixture's directory to the lifecycle owner. It is
    /// deleted only after the owned supervisor has reaped the database process.
    pub async fn retain_directory(
        &self,
        directory: impl Into<Arc<tempfile::TempDir>>,
    ) -> Result<()> {
        let directory = directory.into();
        let ancestor =
            Directory::open(directory.path(), Privacy::Inherited, NameRetention::Movable)?;
        ensure!(
            files::directory(&self.0.directory)?.is_within(&ancestor)?,
            "retained directory does not own memory storage"
        );
        let mut owner = self.0.owner.lock().await;
        let owner = owner
            .as_mut()
            .context("only the sidecar owner can retain a temporary directory")?;
        ensure!(
            owner.retained.is_none(),
            "temporary memory directory already retained"
        );
        owner.retained = Some(directory);
        Ok(())
    }
}

async fn finish_owner(owner: &mut Owner) -> Result<()> {
    #[cfg(windows)]
    if let Some(lifetime) = owner.lifetime.as_mut() {
        lifetime.close(KILL_GRACE).await?;
    }
    drop(owner.lifetime.take());
    let deadline = Instant::now() + CLOSE_GRACE + KILL_GRACE + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = owner
            .child
            .as_mut()
            .context("memory supervisor already reaped")?
            .try_wait()?
        {
            break status;
        }
        ensure!(
            Instant::now() < deadline,
            "memory supervisor shutdown deadline exceeded; lifecycle lock remains authoritative"
        );
        sleep(Duration::from_millis(20)).await;
    };
    owner.child.take();
    ensure!(
        status.success(),
        "memory supervisor exited unsuccessfully ({status})"
    );
    Ok(())
}

fn validate_branch(branch: &str) -> Result<()> {
    ensure!(
        !branch.is_empty()
            && branch.len() <= 128
            && branch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
        "invalid memory branch"
    );
    Ok(())
}

fn private_metadata(path: &Path, directory: bool) -> Result<fs::Metadata> {
    if directory {
        files::directory(path)?;
        Ok(fs::symlink_metadata(path)?)
    } else {
        let (_directory, file) = files::read(path, Privacy::OwnerOnly)?;
        Ok(file.metadata()?)
    }
}

fn private_directory(path: &Path) -> Result<()> {
    files::private_dir(path)
}

/// The external Windows lock has no descendant file handle in the moved store.
#[derive(Debug)]
pub(crate) struct LifecycleLease {
    pub(crate) directory: Directory,
    lock_directory: Directory,
    lock_name: std::ffi::OsString,
    lock: File,
}

impl std::ops::Deref for LifecycleLease {
    type Target = File;
    fn deref(&self) -> &File {
        &self.lock
    }
}

impl LifecycleLease {
    fn validate_root(directory: &Path, root: Option<&Path>) -> Result<()> {
        #[cfg(unix)]
        {
            let _ = directory;
            ensure!(
                root.is_none(),
                "Unix memory preserves its in-directory lifecycle lock"
            );
        }
        #[cfg(windows)]
        {
            let root =
                root.context("Windows memory requires an explicit external lifecycle_root")?;
            ensure!(
                root.is_absolute(),
                "Windows lifecycle_root must be absolute"
            );
            let store = files::directory(directory)?;
            let root = Directory::ensure_private(root)?;
            ensure!(
                !root.is_within(&store)?,
                "Windows lifecycle_root must not lie inside the moved memory store"
            );
        }
        Ok(())
    }

    fn new(directory: &Path, root: Option<&Path>) -> Result<Self> {
        Self::validate_root(directory, root)?;
        let directory = files::directory(directory)?;
        #[cfg(unix)]
        let (lock_directory, lock_name): (_, std::ffi::OsString) =
            (files::directory(directory.path())?, "lifecycle.lock".into());
        #[cfg(windows)]
        let (lock_directory, lock_name): (_, std::ffi::OsString) = {
            let root = Directory::open(
                root.expect("validated Windows lifecycle root"),
                Privacy::OwnerOnly,
                NameRetention::Pinned,
            )?;
            let key: String = directory
                .identity()
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            (root, format!("{key}.lock").into())
        };
        let lock = lock_directory.lock_file(&lock_name)?;
        let lease = Self {
            directory,
            lock_directory,
            lock_name,
            lock,
        };
        lease.verify()?;
        Ok(lease)
    }

    fn verify(&self) -> Result<()> {
        let named = files::directory(self.directory.path())
            .context("memory lifecycle directory was moved or replaced")?;
        ensure!(
            named.identity() == self.directory.identity(),
            "memory lifecycle directory was moved or replaced"
        );
        self.lock_directory
            .verify(&self.lock_name, &self.lock)
            .context("memory lifecycle lock was moved or replaced")?;
        Ok(())
    }

    pub(crate) fn move_to(&mut self, destination: &Path) -> Result<()> {
        self.verify()?;
        self.directory = files::move_directory(&self.directory, destination)?;
        #[cfg(unix)]
        {
            self.lock_directory = files::directory(self.directory.path())?;
        }
        self.verify()
    }

    #[cfg(test)]
    pub(crate) fn move_to_observed(
        &mut self,
        destination: &Path,
        observer: impl FnOnce(&Directory) -> Result<()>,
    ) -> Result<()> {
        self.verify()?;
        self.directory = files::move_directory_observed(&self.directory, destination, observer)?;
        #[cfg(unix)]
        {
            self.lock_directory = files::directory(self.directory.path())?;
        }
        self.verify()
    }
}

fn prepare_directory(directory: &Path, read_only: bool) -> Result<()> {
    if read_only {
        private_metadata(directory, true).context("memory has not been initialized privately")?;
    } else {
        private_directory(directory)?;
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("unrecognized memory directory entry")?;
        let is_directory = matches!(name, "data" | "config" | "home" | "staging");
        ensure!(
            is_directory
                || matches!(
                    name,
                    "identity.json"
                        | "endpoint.json"
                        | "lifecycle.lock"
                        | "server.yaml"
                        | "server.log"
                        | "migration.json"
                        | "ready.json"
                ),
            "unrecognized memory directory entry: {name}"
        );
        private_metadata(&entry.path(), is_directory)?;
    }
    Ok(())
}

fn read_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => {
            result?;
        }
    }
    private_metadata(path, false)?;
    let bytes = files::read_bytes(path, RECORD_LIMIT as u64)?;
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory record exceeds size limit"
    );
    serde_json::from_slice(&bytes)
        .map(Some)
        .context("invalid private memory record")
}

fn write_record(path: &Path, record: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(record)?;
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory record exceeds size limit"
    );
    files::write(path, &bytes)
}

fn load_identity(directory: &Path, project_scope: &str) -> Result<Option<Identity>> {
    let identity: Option<Identity> = read_record(&directory.join("identity.json"))?;
    if let Some(identity) = &identity {
        ensure!(
            identity.version == 1 && identity.project_scope == project_scope,
            "memory project identity mismatch"
        );
        Uuid::parse_str(&identity.instance).context("invalid memory instance identity")?;
        for secret in [&identity.password, &identity.reader_password] {
            ensure!(
                valid_secret(secret),
                "invalid private memory credential record"
            );
        }
    }
    Ok(identity)
}

fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn connect_pool(
    identity: &Identity,
    endpoint: &Endpoint,
    directory: &Path,
    branch: &str,
    read_only: bool,
    max: u32,
) -> Result<MySqlPool> {
    ensure!(
        endpoint.instance == identity.instance && endpoint.port >= 1024,
        "memory endpoint identity mismatch"
    );
    let options = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(endpoint.port)
        .username(if read_only { "kuru_reader" } else { "root" })
        .password(if read_only {
            &identity.reader_password
        } else {
            &identity.password
        })
        .database(&format!("kuru/{branch}"))
        .ssl_mode(MySqlSslMode::Disabled);
    let instance = identity.instance.clone();
    let project_scope = identity.project_scope.clone();
    let expected_directory = directory.join("data");
    MySqlPoolOptions::new()
        .max_connections(max)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(2))
        .idle_timeout(Duration::from_secs(30))
        .after_connect(move |connection, _| {
            let instance = instance.clone();
            let project_scope = project_scope.clone();
            let expected_directory = expected_directory.clone();
            Box::pin(async move {
                let datadir: String = sqlx::query_scalar("SELECT @@datadir")
                    .fetch_one(&mut *connection)
                    .await?;
                if !same_directory(Path::new(&datadir), &expected_directory)
                    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?
                {
                    return Err(sqlx::Error::Protocol(
                        "memory server data directory mismatch".into(),
                    ));
                }
                let row = sqlx::query(
                    "SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1",
                )
                .fetch_one(connection)
                .await?;
                if row.try_get::<String, _>("instance_id")? != instance
                    || row.try_get::<String, _>("project_scope")? != project_scope
                {
                    return Err(sqlx::Error::Protocol(
                        "memory SQL project/instance identity mismatch".into(),
                    ));
                }
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .context("connect to authenticated project memory")
}

async fn verify_identity(pool: &MySqlPool, directory: &Path, identity: &Identity) -> Result<()> {
    timeout(Duration::from_secs(2), async {
        let datadir: String = sqlx::query_scalar("SELECT @@datadir")
            .fetch_one(pool)
            .await?;
        ensure!(
            same_directory(Path::new(&datadir), &directory.join("data"))?,
            "memory server data directory mismatch"
        );
        let row =
            sqlx::query("SELECT instance_id, project_scope FROM kuru_instance WHERE singleton = 1")
                .fetch_one(pool)
                .await?;
        ensure!(
            row.try_get::<String, _>("instance_id")? == identity.instance
                && row.try_get::<String, _>("project_scope")? == identity.project_scope,
            "memory SQL project/instance identity mismatch"
        );
        Ok(())
    })
    .await
    .context("memory identity query deadline exceeded")?
}

async fn live_endpoint(
    directory: &Path,
    identity: &Identity,
    read_only: bool,
) -> Result<Option<Endpoint>> {
    let Some(endpoint): Option<Endpoint> = read_record(&directory.join("endpoint.json"))? else {
        return Ok(None);
    };
    ensure!(
        identity.initialized && endpoint.instance == identity.instance,
        "memory endpoint is not initialized for this instance"
    );
    match timeout(
        Duration::from_millis(250),
        TcpStream::connect(("127.0.0.1", endpoint.port)),
    )
    .await
    {
        Ok(Ok(stream)) => drop(stream),
        _ => return Ok(None),
    }
    let pool = connect_pool(identity, &endpoint, directory, "main", read_only, 1).await?;
    let verified = verify_identity(&pool, directory, identity).await;
    pool.close().await;
    verified?;
    Ok(Some(endpoint))
}

async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(reader: &mut R) -> Result<T> {
    let length = reader
        .read_u32()
        .await
        .context("memory supervisor frame header")? as usize;
    ensure!(
        length > 0 && length <= RECORD_LIMIT,
        "invalid memory supervisor frame length"
    );
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .await
        .context("memory supervisor frame body")?;
    serde_json::from_slice(&bytes).context("invalid memory supervisor frame")
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= RECORD_LIMIT,
        "memory supervisor frame exceeds limit"
    );
    writer.write_u32(bytes.len().try_into()?).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Internal entrypoint shared by the application and package test helper.
pub async fn supervisor_entry() -> Result<()> {
    #[cfg(unix)]
    {
        let mut input =
            pipe::Receiver::from_owned_fd(std::io::stdin().as_fd().try_clone_to_owned()?)?;
        let mut output =
            pipe::Sender::from_owned_fd(std::io::stdout().as_fd().try_clone_to_owned()?)?;
        let request: Request = timeout(Duration::from_secs(5), read_frame(&mut input))
            .await
            .context("memory supervisor configuration deadline exceeded")??;
        supervisor_request(request, &mut input, &mut output).await
    }
    #[cfg(windows)]
    {
        let address = std::env::args_os()
            .nth(2)
            .context("Windows memory supervisor needs its private rendezvous address")?;
        let started = Instant::now();
        let mut channel = pipe::connect(&address, Duration::from_secs(5)).await?;
        let request = timeout(
            Duration::from_secs(5).saturating_sub(started.elapsed()),
            read_frame::<_, Request>(&mut channel),
        )
        .await
        .context("memory supervisor configuration deadline exceeded")??;
        let (mut input, mut output) = tokio::io::split(channel);
        let result = supervisor_request(request, &mut input, &mut output).await;
        let mut channel = input.unsplit(output);
        let closed = channel.close(KILL_GRACE).await;
        result?;
        closed?;
        Ok(())
    }
}

async fn supervisor_request<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<()> {
    if let Err(error) = supervise(request, input, output).await {
        // No credentials are ever serialized in the response. SQL bootstrap
        // credential-setting failures are deliberately reported without SQL text.
        let message = format!("{error:#}");
        let _ = timeout(KILL_GRACE, write_frame(output, &Response::Failed(message))).await;
        return Err(error);
    }
    Ok(())
}

async fn supervise<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<()> {
    ensure!(
        request.timeout_millis > 0 && request.timeout_millis <= 300_000,
        "invalid supervisor startup timeout"
    );
    prepare_directory(&request.directory, request.read_only)?;
    ensure!(
        fs::canonicalize(&request.directory)? == request.directory,
        "memory directory must be canonical"
    );
    let deadline = Instant::now() + Duration::from_millis(request.timeout_millis);
    let lease = LifecycleLease::new(&request.directory, request.lifecycle_root.as_deref())?;
    loop {
        lease.verify()?;
        match lease.lock.try_lock() {
            Ok(()) => {
                lease.verify()?;
                break;
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if request.read_only
                    && let Some(identity) =
                        load_identity(&request.directory, &request.project_scope)?
                    && let Some(endpoint) =
                        live_endpoint(&request.directory, &identity, request.read_only).await?
                {
                    write_frame(
                        output,
                        &Response::Ready {
                            endpoint,
                            owned: false,
                        },
                    )
                    .await?;
                    return Ok(());
                }
                ensure!(
                    Instant::now() < deadline,
                    "memory lifecycle lock is held; refusing takeover"
                );
                let mut byte = [0];
                tokio::select! {
                    result = input.read(&mut byte) => { result?; bail!("memory parent closed during lifecycle acquisition"); }
                    () = sleep(Duration::from_millis(25)) => {}
                }
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    let mut identity = match load_identity(&request.directory, &request.project_scope)? {
        Some(identity) => identity,
        None => {
            ensure!(!request.read_only, "memory has not been initialized");
            ensure!(
                !request.directory.join("data").exists(),
                "unrecognized memory data without identity"
            );
            let identity = Identity {
                version: 1,
                instance: Uuid::new_v4().to_string(),
                project_scope: request.project_scope.clone(),
                password: secret(),
                reader_password: secret(),
                initialized: false,
            };
            write_record(&request.directory.join("identity.json"), &identity)?;
            identity
        }
    };
    ensure!(
        !request.read_only || identity.initialized,
        "memory initialization is incomplete"
    );
    for name in ["data", "config", "home"] {
        private_directory(&request.directory.join(name))?;
    }
    crate::provision::prepare_private_home(&request.directory.join("home"))?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("reserve a private loopback port for Dolt")?;
    let port = listener.local_addr()?.port();
    ensure!(port >= 1024, "unsupported allocated Dolt port");
    drop(listener);
    let endpoint = Endpoint {
        instance: identity.instance.clone(),
        port,
    };
    let yaml = server_yaml(&request.directory, port)?;
    let config_path = request.directory.join("server.yaml");
    write_private(&config_path, yaml.as_bytes())?;
    let mut signals = ShutdownSignals::new()?;
    let mut child = crate::engine::spawn(
        &request.binary,
        &request.directory.join("home"),
        &request.directory,
        vec![
            "sql-server".into(),
            "--config".into(),
            config_path.into_os_string(),
        ],
        vec![
            (
                "DOLT_ROOT_PASSWORD".into(),
                identity.password.clone().into(),
            ),
            ("DOLT_ROOT_HOST".into(), "localhost".into()),
        ],
        true,
    )
    .await?;
    let log = Arc::new(Mutex::new(Vec::new()));
    let stdout = tokio::spawn(drain(
        child.stdout().context("Dolt stdout missing")?,
        log.clone(),
    ));
    let stderr = tokio::spawn(drain(
        child.stderr().context("Dolt stderr missing")?,
        log.clone(),
    ));
    let mut byte = [0];
    let run_result = async {
        tokio::select! {
            ready = start_database(&mut child, &request.directory, &mut identity, &endpoint, deadline) => ready,
            result = input.read(&mut byte) => { result?; Err(anyhow!("memory parent closed during startup")) },
            _ = signals.recv() => Err(anyhow!("memory supervisor terminated during startup")),
        }?;
        write_record(&request.directory.join("endpoint.json"), &endpoint)?;
        write_frame(output, &Response::Ready { endpoint: endpoint.clone(), owned: true }).await?;
        tokio::select! {
            status = child.wait() => { let status = status?; ensure!(status.success(), "Dolt exited unexpectedly ({status})"); Ok(()) },
            result = input.read(&mut byte) => { result?; Ok(()) },
            _ = signals.recv() => Ok(()),
        }
    }.await;
    let stopped = stop_child(&mut child).await;
    if stopped.is_err() {
        // The parent has its own bounded close deadline. A failure to reap
        // cannot make the directory safe to move: retain this supervisor and
        // its lifecycle lease until the actual owned process has terminated.
        eprintln!("Dolt cleanup is delayed; retaining the memory lifecycle lease");
        observe_dolt(&mut child, Child::try_wait).await;
    }
    let _ = timeout(Duration::from_secs(1), async {
        let _ = stdout.await;
        let _ = stderr.await;
    })
    .await;
    let diagnostic = String::from_utf8_lossy(&log.lock().await)
        .replace(&identity.password, "[redacted]")
        .replace(&identity.reader_password, "[redacted]");
    write_private(&request.directory.join("server.log"), diagnostic.as_bytes())?;
    if let Some(published) = read_record::<Endpoint>(&request.directory.join("endpoint.json"))?
        && published.instance == endpoint.instance
        && published.port == endpoint.port
    {
        retire_endpoint(&request.directory)?;
    }
    stopped?;
    run_result.with_context(|| {
        format!(
            "Dolt startup/lifetime failed; private diagnostics: {}",
            request.directory.join("server.log").display()
        )
    })
    // `lease` is released only after child cleanup and endpoint retirement.
}

fn same_directory(actual: &Path, expected: &Path) -> Result<bool> {
    Ok(files::directory(actual)?.identity() == files::directory(expected)?.identity())
}

async fn observe_dolt(
    child: &mut Child,
    mut status: impl FnMut(&mut Child) -> std::io::Result<Option<std::process::ExitStatus>>,
) {
    // The caller retains the lifecycle lease throughout this observation.
    // NativeChild only reports exit once its owned Job has no live processes.
    loop {
        if matches!(status(child), Ok(Some(_))) {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
}

fn retire_endpoint(directory: &Path) -> Result<()> {
    let parent = files::directory(directory)?;
    let stage = Directory::ensure_private(&directory.join("staging"))?;
    let source = std::ffi::OsStr::new("endpoint.json");
    let file = parent.read(source)?;
    let retired = format!("endpoint-{}.retired", Uuid::new_v4());
    stage.rename_file(
        &parent,
        source,
        &file,
        std::ffi::OsStr::new(&retired),
        kuru_platform::fs::Publication::New,
    )?;
    stage.remove_file(std::ffi::OsStr::new(&retired), file)?;
    Ok(())
}

#[cfg(unix)]
struct ShutdownSignals {
    termination: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}
#[cfg(windows)]
struct ShutdownSignals {
    termination: tokio::signal::windows::CtrlBreak,
    interrupt: tokio::signal::windows::CtrlC,
}
impl ShutdownSignals {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                termination: signal(SignalKind::terminate())?,
                interrupt: signal(SignalKind::interrupt())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                termination: tokio::signal::windows::ctrl_break()?,
                interrupt: tokio::signal::windows::ctrl_c()?,
            })
        }
    }
    async fn recv(&mut self) {
        tokio::select! { _ = self.termination.recv() => {}, _ = self.interrupt.recv() => {} }
    }
}

fn server_yaml(directory: &Path, port: u16) -> Result<String> {
    // Dolt 2.3.3 does not derive @@datadir from data_dir. Initialize its
    // read-only SQL variable from the same canonical path for identity probes.
    let quoted = |name: &str| serde_json::to_string(&directory.join(name));
    Ok(format!(
        "log_level: warning\nlog_format: text\nbehavior:\n  autocommit: true\n  dolt_transaction_commit: false\n  event_scheduler: \"OFF\"\n  auto_gc_behavior:\n    enable: false\nlistener:\n  host: 127.0.0.1\n  port: {port}\n  max_connections: 32\n  max_connections_timeout_millis: 1000\n  read_timeout_millis: 5000\n  write_timeout_millis: 5000\n  allow_cleartext_passwords: false\ndata_dir: {}\ncfg_dir: {}\nprivilege_file: {}\nbranch_control_file: {}\nsystem_variables:\n  datadir: {}\n  secure_file_priv: {}\n",
        quoted("data")?,
        quoted("config")?,
        quoted("config/privileges.db")?,
        quoted("config/branch_control.db")?,
        quoted("data")?,
        quoted("file-operations-disabled")?
    ))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    files::write(path, bytes)
}

async fn drain<R: AsyncRead + Unpin>(mut reader: R, log: Arc<Mutex<Vec<u8>>>) {
    let mut bytes = [0; 4096];
    while let Ok(length) = reader.read(&mut bytes).await {
        if length == 0 {
            break;
        }
        let mut log = log.lock().await;
        log.extend_from_slice(&bytes[..length]);
        if log.len() > LOG_LIMIT {
            let remove = log.len() - LOG_LIMIT;
            log.drain(..remove);
        }
    }
}

async fn start_database(
    child: &mut Child,
    directory: &Path,
    identity: &mut Identity,
    endpoint: &Endpoint,
    deadline: Instant,
) -> Result<()> {
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("Dolt exited before readiness ({status})");
        }
        ensure!(
            Instant::now() < deadline,
            "authenticated Dolt startup deadline exceeded"
        );
        let options = MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(endpoint.port)
            .username("root")
            .password(&identity.password)
            .ssl_mode(MySqlSslMode::Disabled);
        if let Ok(Ok(pool)) = timeout(
            Duration::from_millis(400),
            MySqlPoolOptions::new()
                .max_connections(1)
                .connect_with(options),
        )
        .await
        {
            let initialized = timeout(
                deadline.saturating_duration_since(Instant::now()),
                initialize_database(&pool, directory, identity),
            )
            .await;
            pool.close().await;
            return initialized.context("Dolt database bootstrap deadline exceeded")?;
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn initialize_database(
    pool: &MySqlPool,
    directory: &Path,
    identity: &mut Identity,
) -> Result<()> {
    let datadir: String = sqlx::query_scalar("SELECT @@datadir")
        .fetch_one(pool)
        .await?;
    ensure!(
        same_directory(Path::new(&datadir), &directory.join("data"))
            .with_context(|| format!("resolve Dolt bootstrap datadir {datadir:?}"))?,
        "Dolt bootstrap data directory mismatch"
    );
    if !identity.initialized {
        sqlx::query("CREATE DATABASE IF NOT EXISTS kuru")
            .execute(pool)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS kuru.kuru_instance (singleton TINYINT PRIMARY KEY, instance_id VARCHAR(36) NOT NULL, project_scope TEXT NOT NULL)").execute(pool).await?;
        let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru.kuru_instance")
            .fetch_one(pool)
            .await?;
        if existing == 0 {
            sqlx::query("INSERT INTO kuru.kuru_instance (singleton, instance_id, project_scope) VALUES (1, ?, ?)").bind(&identity.instance).bind(&identity.project_scope).execute(pool).await?;
        }
        // Dolt's CREATE USER parser rejects bind parameters. Only our generated
        // 64-character hex credential can enter this literal; never user input.
        ensure!(
            valid_secret(&identity.reader_password),
            "invalid private reader credential"
        );
        let credential_sql = format!(
            "CREATE USER IF NOT EXISTS 'kuru_reader'@'localhost' IDENTIFIED BY '{}'",
            identity.reader_password
        );
        sqlx::query(sqlx::AssertSqlSafe(credential_sql))
            .execute(pool)
            .await
            .map_err(|_| anyhow!("configuring private read-only memory account failed"))?;
        sqlx::query("GRANT SELECT ON kuru.* TO 'kuru_reader'@'localhost'")
            .execute(pool)
            .await?;
    }
    let row = sqlx::query(
        "SELECT instance_id, project_scope FROM kuru.kuru_instance WHERE singleton = 1",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        row.try_get::<String, _>("instance_id")? == identity.instance
            && row.try_get::<String, _>("project_scope")? == identity.project_scope,
        "Dolt bootstrap project identity mismatch"
    );
    if !identity.initialized {
        identity.initialized = true;
        write_record(&directory.join("identity.json"), identity)?;
    }
    Ok(())
}

async fn stop_child(child: &mut Child) -> Result<()> {
    child.stop(CLOSE_GRACE, KILL_GRACE).await
}

#[cfg(all(test, unix))]
#[path = "server_tests.rs"]
mod tests;
#[cfg(all(test, windows))]
#[path = "server/windows_tests.rs"]
mod windows_tests;

#[cfg(windows)]
pub(crate) mod windows_fixture;
