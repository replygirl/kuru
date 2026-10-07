//! Private local protocol for the project memory owner.
//!
//! This module contains no provider, tool or conversation control operation.
//! Transport privacy excludes other OS users; a same-user process able to read
//! the private endpoint record is within the account's local authority.

use std::sync::Arc;
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, PublicationPhase};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::open_timeline;
use crate::progress::{MemoryOpenStage, ProgressReporter};

pub(crate) mod activity;
pub(crate) mod rpc;
pub use rpc::{ServiceCall, ServiceReply, ServiceRequest, ServiceResponse, ServiceValue};

pub const PROTOCOL_MAJOR: u16 = 1;
// Exact-ref recovery and session-provenance calls require this owner version.
// Older owners reject the new client before a mutating frame.
pub const PROTOCOL_MINOR: u16 = 13;
pub const HANDSHAKE_LIMIT: usize = 16 * 1024;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest single accept wait before the serve loop re-verifies its owner
/// lock, while attachments are live or while a new owner awaits its starter.
const OWNER_LOCK_RECHECK_INTERVAL: Duration = Duration::from_secs(35);
const MAX_ATTACHMENTS: usize = 32;
/// How long a starting client sleeps between attach attempts while it waits
/// for the owner it just spawned. A cadence, not a deadline: the readiness
/// wait ends when the owner's activity record has not changed for
/// `memory.startup_timeout_secs`, or at once when the owner exits, retires
/// that record or marks it failing. The owner binds its listener before
/// publishing its endpoint record, so the poll after the record appears
/// attaches, and the open waits about half this interval past readiness. A
/// miss before publication costs one small private-file read, a non-blocking
/// child status check and one activity record read, observed or not. A Unix
/// connect without a listener is refused at once. A Windows pipe connect
/// retries an absent or busy pipe inside one attempt every 5 ms
/// (`kuru_platform::windows::pipe`) until its own deadline; attempts are
/// serial, so this only adds a sleep after that connect returns, and at twice
/// the pipe's retry interval never polls finer than it.
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(feature = "test-support")]
const STARTUP_STAGE_DIAGNOSTIC_ENV: &str = "KURU_TEST_MEMORY_STARTUP_STAGES";

/// Test-support hook: names an existing private file that receives the
/// stderr of any owner this process spawns without a caller-supplied stderr.
/// A test sets it on the CLI children it runs, so the owner a child elects
/// logs where the test can read it. Inert when unset.
#[cfg(feature = "test-support")]
pub const OWNER_DIAGNOSTIC_ENV: &str = "KURU_TEST_MEMORY_OWNER_DIAGNOSTIC";

/// The owner's diagnostic file named by [`OWNER_DIAGNOSTIC_ENV`], opened for
/// append without creating it, so a child cannot invent a log location.
#[cfg(feature = "test-support")]
fn fixture_owner_diagnostic() -> Result<Option<File>> {
    std::env::var_os(OWNER_DIAGNOSTIC_ENV)
        .filter(|path| !path.is_empty())
        .map(|path| {
            File::options().append(true).open(&path).with_context(|| {
                format!("open test owner diagnostic {}", Path::new(&path).display())
            })
        })
        .transpose()
}

/// The most of a private startup diagnostic that a failed start forwards.
#[cfg(feature = "test-support")]
const STARTUP_DIAGNOSTIC_FORWARD_LIMIT: u64 = 1024 * 1024;

/// Append a failed start's private startup diagnostic, which holds its
/// owner's stderr, to `forward`, the owner diagnostic file a test named.
/// Best effort and bounded: it never changes the start's outcome.
#[cfg(feature = "test-support")]
fn forward_startup_diagnostic(diagnostic: &mut File, forward: &mut File) {
    use std::io::Seek as _;

    if diagnostic.rewind().is_ok() {
        let _ = io::copy(
            &mut (&*diagnostic).take(STARTUP_DIAGNOSTIC_FORWARD_LIMIT),
            forward,
        );
    }
}

#[cfg(feature = "test-support")]
pub(crate) fn fixture_startup_stages_enabled() -> bool {
    std::env::var_os(STARTUP_STAGE_DIAGNOSTIC_ENV).as_deref() == Some(OsStr::new("1"))
}

#[cfg(feature = "test-support")]
fn fixture_startup_observations(
    options: &crate::store::OpenOptions,
    diagnostic: &mut File,
) -> String {
    use std::io::Seek as _;

    let last_stage = diagnostic
        .rewind()
        .ok()
        .and_then(|()| {
            let mut contents = String::new();
            diagnostic.take(4096).read_to_string(&mut contents).ok()?;
            contents
                .lines()
                .filter_map(|line| line.strip_prefix("memory startup stage: "))
                .rfind(|stage| {
                    matches!(
                        *stage,
                        "WaitingForProjectOwnership"
                            | "WaitingForRuntimeCache"
                            | "VerifyingRuntimeCache"
                            | "ExtractingEmbeddedRuntime"
                            | "CheckingRuntimeVersion"
                            | "PreparingDatabase"
                            | "OpeningDatabase"
                            | "Ready"
                            | "CreatingDatabase"
                            | "UpgradingDatabase"
                            | "StartingMemoryService"
                    )
                })
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "none".into());
    let owner = match ServiceLock::try_acquire(
        &options.data_dir,
        &options.project_scope,
        ServiceLockKind::Owner,
    ) {
        Ok(Some(lock)) => match lock.release() {
            Ok(()) => "free",
            Err(_) => "observation-failed",
        },
        Ok(None) => "held",
        Err(_) => "observation-failed",
    };
    let endpoint = match EndpointRecord::read(&options.data_dir, &options.project_scope) {
        Ok(Some(_)) => "present",
        Ok(None) => "absent",
        Err(_) => "observation-failed",
    };
    format!("test startup observations: stage={last_stage}; owner={owner}; endpoint={endpoint}")
}

/// Internal process entry used by both the ordinary executable and native
/// fixtures. Paths arrive as native OS arguments so non-UTF-8 names survive.
pub async fn service_entry(arguments: impl IntoIterator<Item = OsString>) -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(release) = std::env::var_os(SERVICE_STAND_IN_ENV).filter(|path| !path.is_empty()) {
        return stand_in_owner(&PathBuf::from(release)).await;
    }
    crate::open_timeline::install_from_env();
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
    // Optional: a starter from before the token existed passes nine.
    let starter_token = arguments
        .next()
        .map(|token| {
            if token == "-" {
                return Ok(None);
            }
            token
                .to_str()
                .and_then(|token| uuid::Uuid::parse_str(token).ok())
                .context("invalid service starter token")
                .map(Some)
        })
        .transpose()?
        .flatten();
    let expected_instance = arguments
        .next()
        .map(|instance| {
            if instance == "-" {
                return Ok(None);
            }
            let instance = instance
                .into_string()
                .map_err(|_| anyhow::anyhow!("expected memory instance is not UTF-8"))?;
            ensure!(
                uuid::Uuid::parse_str(&instance)?.to_string() == instance,
                "invalid expected memory instance"
            );
            Ok::<_, anyhow::Error>(Some(instance))
        })
        .transpose()?
        .flatten();
    let service_idle_timeout_secs = arguments
        .next()
        .map(|idle| {
            idle.to_str()
                .context("service idle timeout is not UTF-8")?
                .parse::<u64>()
                .context("invalid service idle timeout")
        })
        .transpose()?
        .unwrap_or_else(|| kuru_core::MemoryConfig::default().service_idle_timeout_secs);
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
    options.config.service_idle_timeout_secs = service_idle_timeout_secs;
    options.config.validate()?;
    options.supervisor = supervisor;
    options.starter_token = starter_token;
    options.expected_instance = expected_instance;
    Ok((project, options))
}

fn optional_path(argument: OsString) -> Option<PathBuf> {
    (argument != OsStr::new("-")).then(|| PathBuf::from(argument))
}

/// Test-support stand-in owner: names a release file. An owner process
/// started with it takes no lock, opens nothing and writes nothing; it stays
/// alive until the file holds a decimal exit status, then exits with that
/// status. A test passes it only through the task-local owner environment,
/// so its readiness tests drive the starter against a real child process on
/// every platform while publishing the owner's record itself.
#[cfg(feature = "test-support")]
pub const SERVICE_STAND_IN_ENV: &str = "KURU_TEST_MEMORY_SERVICE_STAND_IN";

/// The stand-in owner's whole life. It polls in its own fixture process,
/// as `client_fixture_entry` does, every 10 ms of real time. A missing or
/// not yet complete file is not a release. It consumes a release by
/// removing the file and then exits at once, so its fixture can wait for
/// that removal before the test ends: an instrumented stand-in that outlived
/// its test would write its coverage profile after the partition's tests.
/// The 120 s bound is only a backstop for a fixture that never releases it.
#[cfg(feature = "test-support")]
async fn stand_in_owner(release: &Path) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        match std::fs::read_to_string(release) {
            Ok(text) => {
                if let Ok(status) = text.trim().parse::<i32>() {
                    std::fs::remove_file(release)
                        .context("consume memory service stand-in release")?;
                    std::process::exit(status);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("read memory service stand-in release");
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service stand-in was not released"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
    // Report the attached generation, then keep the attachment until the
    // caller closes stdin, so its sibling starter meets a live owner.
    {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", client.generation())?;
        stdout.flush()?;
    }
    tokio::task::spawn_blocking(|| {
        use std::io::Read;
        std::io::stdin().read_to_end(&mut Vec::new())
    })
    .await??;
    drop(client);
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

/// One live attachment. An open attachment keeps the owner running; a failed
/// exchange invalidates the stream for reuse rather than replaying a write.
pub struct ServiceAttachment {
    stream: Option<LocalStream>,
    /// A stream whose exchange did not complete, kept open but never used
    /// again, so the owner still counts this client until a replacement
    /// connection has been installed. Only with `retain_after_abandon`.
    held: Option<LocalStream>,
    retain_after_abandon: bool,
    identity: AttachmentIdentity,
    authority: EndpointAuthority,
    locator: Option<AttachmentLocator>,
    last_fault: Option<rpc::ServiceFault>,
    #[cfg(any(test, feature = "test-support"))]
    reply_pause: Option<Arc<rpc::ReplyPause>>,
}

/// Owns a stream for the length of one exchange. Unless the exchange
/// completes, dropping it (on an error or a cancelled call) parks the stream
/// in `held` when the attachment retains abandoned streams, and otherwise
/// closes it.
struct InFlight<'a> {
    stream: Option<LocalStream>,
    held: &'a mut Option<LocalStream>,
    retain: bool,
}

impl InFlight<'_> {
    fn stream(&mut self) -> &mut LocalStream {
        self.stream.as_mut().expect("stream in flight")
    }

    fn complete(mut self) -> LocalStream {
        self.stream.take().expect("stream in flight")
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take()
            && self.retain
        {
            *self.held = Some(stream);
        }
    }
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
    identity: AttachmentIdentity,
}

#[derive(Clone)]
struct AttachmentIdentity {
    client: uuid::Uuid,
    proof: Arc<std::sync::Mutex<Option<crate::SessionDriverProof>>>,
}

impl AttachmentIdentity {
    fn new() -> Self {
        Self {
            client: uuid::Uuid::new_v4(),
            proof: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

impl AttachmentFactory {
    /// Keep one logical driver's identity across an independently checked
    /// successor endpoint. This adopts no old generation as new authority.
    pub(crate) fn for_checked_successor_of(&self, previous: &Self) -> Result<Self> {
        ensure!(
            self.authority.project_path == previous.authority.project_path
                && self.authority.project_scope == previous.authority.project_scope
                && self.authority.history_scope == previous.authority.history_scope
                && self.authority.store_instance == previous.authority.store_instance,
            "memory identity changed before logical client reattachment"
        );
        Ok(Self {
            identity: previous.identity.clone(),
            ..self.clone()
        })
    }

    /// A Harness cloned from an existing view still has its own driver.
    pub(crate) fn for_new_driver(&self) -> Self {
        Self {
            identity: AttachmentIdentity::new(),
            ..self.clone()
        }
    }

    pub(crate) fn publish_driver_proof(
        &self,
        proof: Option<crate::SessionDriverProof>,
    ) -> Result<()> {
        *self
            .identity
            .proof
            .lock()
            .map_err(|_| anyhow::anyhow!("driver proof state is unavailable"))? = proof;
        Ok(())
    }
    pub(crate) fn restored_history_scope(&self) -> Option<&str> {
        self.authority.history_scope.as_deref()
    }

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
        connect_handshake_identified(
            &mut stream,
            &self.authority,
            None,
            Some(self.identity.client),
        )
        .await?;
        Ok(ServiceAttachment {
            stream: Some(stream),
            held: None,
            retain_after_abandon: false,
            identity: self.identity.clone(),
            authority: self.authority.clone(),
            locator: Some(self.locator.clone()),
            last_fault: None,
            #[cfg(any(test, feature = "test-support"))]
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

    /// A dedicated idle presence connection receives no unsolicited frames.
    /// Only an actual read outcome, never a polling deadline, signals loss.
    pub(crate) async fn wait_for_presence_loss(&mut self) -> Result<()> {
        let stream = self
            .stream
            .as_mut()
            .context("presence attachment is closed")?;
        let mut byte = [0_u8; 1];
        let count = stream.read(&mut byte).await?;
        ensure!(count == 0, "unexpected frame on idle presence attachment");
        Ok(())
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
        let proof = self
            .identity
            .proof
            .lock()
            .map_err(|_| anyhow::anyhow!("driver proof state is unavailable"))?
            .clone();
        let mut envelope = rpc::ServiceRequest::with_id(self.generation(), id, call);
        envelope.driver = proof.clone();
        self.last_fault = None;
        ensure!(
            rpc::encoded_bytes(&envelope)? <= rpc::OPERATION_FRAME_LIMIT,
            "managed operation request exceeds its exact frame envelope"
        );
        let call = envelope.call;
        let stream = self
            .stream
            .take()
            .context("memory service attachment is closed")?;
        self.last_fault = None;
        // A live stream and a held one never coexist: after an incomplete
        // exchange this attachment has no stream until it is replaced.
        let mut in_flight = InFlight {
            stream: Some(stream),
            held: &mut self.held,
            retain: self.retain_after_abandon,
        };
        #[cfg(any(test, feature = "test-support"))]
        let response = if let Some(pause) = self.reply_pause.take() {
            rpc::exchange_attached_with_id_paused(
                in_flight.stream(),
                &self.authority,
                id,
                call,
                &pause,
                proof,
            )
            .await?
        } else {
            rpc::exchange_attached_for_driver(in_flight.stream(), &self.authority, id, call, proof)
                .await?
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let response =
            rpc::exchange_attached_for_driver(in_flight.stream(), &self.authority, id, call, proof)
                .await?;
        let stream = in_flight.complete();
        self.last_fault = match &response {
            rpc::ServiceResponse::Rejected(fault) => Some(fault.clone()),
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
            identity: self.identity.clone(),
        })
    }

    /// Close the live stream and any held one. A held stream left in a
    /// retained clone of this attachment would otherwise keep the owner
    /// running after an explicit close.
    pub fn close(&mut self) {
        self.stream = None;
        self.held = None;
    }

    /// Keep the stream of an incomplete exchange open, unused, until this
    /// attachment is replaced (Option B for a writable session's primary).
    pub(crate) fn retain_after_abandon(&mut self) {
        self.retain_after_abandon = true;
    }

    pub(crate) fn retains_after_abandon(&self) -> bool {
        self.retain_after_abandon
    }

    pub(crate) fn holds_abandoned_stream(&self) -> bool {
        self.held.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn pause_after_next_send(&mut self, pause: Arc<rpc::ReplyPause>) {
        self.reply_pause = Some(pause);
    }
}

/// How the client's startup wait was spent, from instants it already
/// takes on its own path. Offsets are measured from one start instant and then
/// differenced, so the rounded phases sum to the whole elapsed wait. The owner
/// reports no stage timing to the client; this covers only what the client
/// observes itself. It is built only after the child was just seen running.
/// The owner may report open stages through the activity record; see
/// `service/activity.rs`.
#[derive(Debug)]
struct ReadinessSplit {
    started: tokio::time::Instant,
    elected: tokio::time::Instant,
    probed: tokio::time::Instant,
    spawned: tokio::time::Instant,
    expired: tokio::time::Instant,
    polls: u32,
    last_attach: AttachMiss,
}

impl std::fmt::Display for ReadinessSplit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let offset = |instant: tokio::time::Instant| {
            instant.saturating_duration_since(self.started).as_millis()
        };
        let (elected, probed, spawned, expired) = (
            offset(self.elected),
            offset(self.probed),
            offset(self.spawned),
            offset(self.expired),
        );
        write!(
            formatter,
            "client phases: election={elected}ms; owner-probe={}ms; spawn={}ms; readiness={}ms; polls={}; child=running; last-attach={}",
            probed.saturating_sub(elected),
            spawned.saturating_sub(probed),
            expired.saturating_sub(spawned),
            self.polls,
            self.last_attach.as_str(),
        )
    }
}

/// Why the elected starter's wait for the owner it spawned ended without
/// readiness. Each variant carries the owner progress the starter observed:
/// the last published stage it read, the owner's progress count in the last
/// record it read, and the time since it last read a change that was
/// progress (since spawn when it read none). A failing mark is not progress. The
/// leading texts are distinct and none is a prefix of another; the
/// `owner progress:` clause follows the leading text and precedes any test
/// observations and the client phase split.
#[derive(Debug)]
enum ReadinessFailure {
    /// No progress for one window.
    Stalled {
        last_stage: Option<MemoryOpenStage>,
        seen: u64,
        since: Duration,
        window: Duration,
        observations: Option<String>,
        /// A gated starter's bounded read of its own owner's stream, as
        /// `owner timeline: ...; `, or empty.
        owner_timeline: String,
        split: ReadinessSplit,
    },
    /// The owner process exited before readiness.
    OwnerExited {
        status: std::process::ExitStatus,
        last_stage: Option<MemoryOpenStage>,
        seen: u64,
        since: Duration,
    },
    /// A record this starter had read left its name before attachment: the
    /// owner ended its open without serving this starter.
    OwnerRetired {
        last_stage: Option<MemoryOpenStage>,
        seen: u64,
        since: Duration,
    },
    /// The owner marked its record failing, with the reason it recorded.
    OwnerFailed {
        reason: String,
        last_stage: Option<MemoryOpenStage>,
        seen: u64,
        since: Duration,
    },
}

impl std::fmt::Display for ReadinessFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let progress = |formatter: &mut std::fmt::Formatter<'_>,
                        last_stage: &Option<MemoryOpenStage>,
                        seen: u64,
                        since: Duration| {
            write!(
                formatter,
                "owner progress: last={} advances={seen} since-progress={}ms",
                last_stage.and_then(activity::stage_name).unwrap_or("none"),
                since.as_millis(),
            )
        };
        match self {
            Self::Stalled {
                last_stage,
                seen,
                since,
                window,
                observations,
                owner_timeline,
                split,
            } => {
                formatter.write_str("memory service readiness deadline exceeded; ")?;
                progress(formatter, last_stage, *seen, *since)?;
                write!(formatter, " window={}ms; ", window.as_millis())?;
                if let Some(observations) = observations {
                    write!(formatter, "{observations}; ")?;
                }
                write!(formatter, "{owner_timeline}{split}")
            }
            Self::OwnerExited {
                status,
                last_stage,
                seen,
                since,
            } => {
                write!(
                    formatter,
                    "memory service exited before readiness: {status}; "
                )?;
                progress(formatter, last_stage, *seen, *since)
            }
            Self::OwnerRetired {
                last_stage,
                seen,
                since,
            } => {
                formatter.write_str("memory service ended its open before readiness; ")?;
                progress(formatter, last_stage, *seen, *since)
            }
            Self::OwnerFailed {
                reason,
                last_stage,
                seen,
                since,
            } => {
                write!(
                    formatter,
                    "memory service open failed before readiness: {reason}; "
                )?;
                progress(formatter, last_stage, *seen, *since)
            }
        }
    }
}

impl std::error::Error for ReadinessFailure {}

/// The owner timeline clause a gated starter adds at its readiness deadline:
/// one bounded read of its own owner's stream (named from the activity tag
/// of the token it passed), with no wait and no retry, every error mapped to
/// text. `None` when this starter is ungated: then nothing is read.
///
/// `owner-exec` is the owner's anchor less `spawn_called`, the instant
/// taken just before the spawn call, and `since-last` is `now` less the last
/// line's wall-clock time: wall-clock differences across two processes, so
/// a clock step skews them. An owner anchored before `spawn_called` is a
/// predecessor's file with the same tag, reported only as `stale`; that
/// comparison is at the platform clock's resolution (100 ns on Windows), so
/// an anchor within one tick of `spawn_called` is not stale. Lines
/// are `<name> <offset-ns> <unix-ns>`; a trailing partial line is still
/// being written and ignored, and a malformed complete line is counted,
/// never named. The clause never carries a path, token, scope or tag.
fn owner_timeline_clause(
    data_dir: &Path,
    scope: &str,
    tag: &str,
    spawn_called: std::time::SystemTime,
    now: std::time::SystemTime,
) -> Option<String> {
    if !open_timeline::gate_set() {
        return None;
    }
    let read = EndpointRecord::directory(data_dir, scope).and_then(|directory| {
        crate::files::read_bytes(
            &directory.join(open_timeline::stream_name(tag)),
            open_timeline::MAX_BYTES as u64,
        )
    });
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(error)
            if error.chain().any(|cause| {
                cause
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
            }) =>
        {
            return Some("owner timeline: absent".into());
        }
        Err(_) => return Some("owner timeline: unreadable".into()),
    };
    Some(owner_timeline_text(&bytes, spawn_called, now))
}

fn owner_timeline_text(
    bytes: &[u8],
    spawn_called: std::time::SystemTime,
    now: std::time::SystemTime,
) -> String {
    const MS: u64 = 1_000_000;
    fn unix_ns(time: std::time::SystemTime) -> u64 {
        time.duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            })
    }
    fn number(field: &str) -> Option<u64> {
        field
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| field.parse().ok())
            .flatten()
    }
    fn line(bytes: &[u8]) -> Option<(&str, u64, u64)> {
        let mut fields = std::str::from_utf8(bytes).ok()?.split(' ');
        let (name, offset, unix) = (fields.next()?, fields.next()?, fields.next()?);
        let named = (1..=32).contains(&name.len())
            && name
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        (named && fields.next().is_none()).then_some(())?;
        Some((name, number(offset)?, number(unix)?))
    }
    let mut events = Vec::new();
    let mut skipped = 0_usize;
    // Everything after the last newline is a line still being written.
    if let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') {
        for raw in bytes[..end].split(|byte| *byte == b'\n') {
            match line(raw) {
                Some(event) => events.push(event),
                None => skipped += 1,
            }
        }
    }
    let skipped = if skipped == 0 {
        String::new()
    } else {
        format!("; skipped={skipped}")
    };
    let (Some(&(_, first_offset, first_unix)), Some(&(last, last_offset, last_unix))) =
        (events.first(), events.last())
    else {
        return format!("owner timeline: empty{skipped}");
    };
    let anchor = first_unix.saturating_sub(first_offset);
    let spawned = unix_ns(spawn_called);
    if anchor < spawned {
        return "owner timeline: stale".into();
    }
    let listed = events
        .iter()
        .map(|(name, offset, _)| format!("{name}={}", offset / MS))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "owner timeline: owner-exec={}ms; {listed} (ms); last={last} +{}ms; since-last={}ms{skipped}",
        (anchor - spawned) / MS,
        last_offset / MS,
        unix_ns(now).saturating_sub(last_unix) / MS,
    )
}

/// Attach to a valid owner, or elect and start one while retaining a distinct
/// start lock through readiness or the end of the readiness wait. Election
/// and the wait for a previous owner stay bounded by one
/// `startup_timeout_secs` from `started`; the wait for the spawned owner is
/// bounded by that owner's progress. Endpoint readiness is the authenticated private handshake;
/// the child holds the owner lock before publishing it. `progress` receives
/// only the stages this client observes or forwards from its own owner.
pub(crate) async fn attach_or_start_observed(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    progress: &mut ProgressReporter,
) -> Result<ServiceAttachment> {
    ensure_project_scope(project, &options.project_scope)?;
    options.config.validate()?;
    ensure!(
        executable.is_absolute(),
        "service executable must be absolute"
    );
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(options.config.startup_timeout_secs);
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
        .await
        .context("attach before memory service start election")?
    {
        return Ok(attached);
    }
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
    let attached =
        attach_or_spawn_elected(options, project, executable, started, deadline, progress).await;
    // Released explicitly on every path: a sibling's child between fork and
    // exec may hold a duplicate of this lock's description.
    let released = start.release();
    let attached = attached?;
    released?;
    Ok(attached)
}

/// The elected part of [`attach_or_start`], run while the caller holds the
/// start lock.
async fn attach_or_spawn_elected(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    started: tokio::time::Instant,
    deadline: tokio::time::Instant,
    progress: &mut ProgressReporter,
) -> Result<ServiceAttachment> {
    let elected = tokio::time::Instant::now();
    if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
        .await
        .context("attach after acquiring memory service start election")?
    {
        return Ok(attached);
    }
    loop {
        // A live owner may still be booting, or reaping Dolt after retiring
        // its endpoint. A busy lock never authorizes another spawn.
        if let Some(owner_probe) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            owner_probe.release()?;
            break;
        }
        // Reached only while another process holds owner authority.
        progress.report(MemoryOpenStage::WaitingForProjectOwnership);
        ensure!(
            tokio::time::Instant::now() < deadline,
            "the previous memory service was still shutting down or did not publish a valid endpoint within memory.startup_timeout_secs"
        );
        if let Some(attached) = try_attach(&options.data_dir, &options.project_scope, project)
            .await
            .context("attach while the elected memory service owner is still active")?
        {
            return Ok(attached);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let probed = tokio::time::Instant::now();
    // Ends any wait shown above: a stage is never reported twice.
    progress.report(MemoryOpenStage::StartingMemoryService);
    #[cfg(feature = "test-support")]
    let mut startup_diagnostic = fixture_startup_stages_enabled()
        .then(|| tempfile::tempfile_in(&options.data_dir))
        .transpose()
        .context("create private test startup diagnostic")?;
    #[cfg(feature = "test-support")]
    let child_stderr = match startup_diagnostic
        .as_ref()
        .map(File::try_clone)
        .transpose()
        .context("reopen private test startup diagnostic")?
    {
        Some(stderr) => Some(stderr),
        None => fixture_owner_diagnostic()?,
    };
    #[cfg(not(feature = "test-support"))]
    let child_stderr = None;
    // Only this starter presents the token, and only to the child it spawns
    // here; a token reused from the session's options cannot reach another
    // owner.
    let starter_token = options.starter_token.unwrap_or_else(uuid::Uuid::new_v4);
    // Only a gated starter reads its owner's stream at the deadline. Taken
    // before the spawn call: a Windows spawn awaits, and its owner may stamp
    // before the call returns, which would read as stale.
    let spawn_called = open_timeline::gate_set().then(std::time::SystemTime::now);
    let mut child = ServiceProcess::new(
        spawn_service(
            options,
            project,
            executable,
            child_stderr,
            Some(starter_token),
        )
        .await
        .context("spawn elected memory service owner")?,
    );
    let spawned = tokio::time::Instant::now();
    let outcome: Result<ServiceAttachment> = async {
        // Every starter reads its own owner's record, observed or not; only an
        // observed open is shown its stages. The first window starts at spawn.
        let mut owner = activity::OwnerWatch::new(activity::activity_tag(&starter_token), spawned);
        let window = Duration::from_secs(options.config.startup_timeout_secs);
        let mut polls: u32 = 0;
        loop {
            polls = polls.saturating_add(1);
            let last_attach = match try_attach_observed(
                &options.data_dir,
                &options.project_scope,
                project,
                Some(starter_token),
            )
            .await
            {
                Ok(Ok(attached)) => return Ok(attached),
                Ok(Err(miss)) => miss,
                Err(error) => {
                    let child_state = match child.try_wait() {
                        Ok(Some(status)) => format!("exited with {status}"),
                        Ok(None) => "remained running".into(),
                        Err(status_error) => {
                            format!("status observation failed with {status_error}")
                        }
                    };
                    return Err(error).with_context(|| {
                        format!(
                            "attach after starting the elected memory service; child {child_state}"
                        )
                    });
                }
            };
            // Exit evidence carries the last stage read at an earlier poll: this
            // poll's read follows the exit check.
            if let Some(status) = child.try_wait()? {
                return Err(ReadinessFailure::OwnerExited {
                    status,
                    last_stage: owner.last_stage(),
                    seen: owner.seen(),
                    since: owner.last_progress().elapsed(),
                }
                .into());
            }
            // One bounded read per poll, after the attach attempt and the exit
            // check and before the window check, so progress first visible at
            // this poll counts at this poll. Only a change of the record moves
            // the window; a missing, foreign, unreadable or unchanged record
            // moves nothing, so a failed publication or read never extends it.
            match owner.poll(&options.data_dir, &options.project_scope, progress) {
                activity::Watched::Waiting => {}
                activity::Watched::Retired => {
                    return Err(ReadinessFailure::OwnerRetired {
                        last_stage: owner.last_stage(),
                        seen: owner.seen(),
                        since: owner.last_progress().elapsed(),
                    }
                    .into());
                }
                activity::Watched::Failing(reason) => {
                    return Err(ReadinessFailure::OwnerFailed {
                        reason,
                        last_stage: owner.last_stage(),
                        seen: owner.seen(),
                        since: owner.last_progress().elapsed(),
                    }
                    .into());
                }
            }
            let now = tokio::time::Instant::now();
            let since = now.saturating_duration_since(owner.last_progress());
            if since >= window {
                // Taken before any failure-only observation below.
                let split = ReadinessSplit {
                    started,
                    elected,
                    probed,
                    spawned,
                    expired: now,
                    polls,
                    last_attach,
                };
                #[cfg(feature = "test-support")]
                let observations = startup_diagnostic
                    .as_mut()
                    .map(|diagnostic| fixture_startup_observations(options, diagnostic));
                #[cfg(not(feature = "test-support"))]
                let observations = None;
                let owner_timeline = spawn_called
                    .and_then(|spawn_called| {
                        owner_timeline_clause(
                            &options.data_dir,
                            &options.project_scope,
                            &activity::activity_tag(&starter_token),
                            spawn_called,
                            std::time::SystemTime::now(),
                        )
                    })
                    .map(|clause| format!("{clause}; "))
                    .unwrap_or_default();
                return Err(ReadinessFailure::Stalled {
                    last_stage: owner.last_stage(),
                    seen: owner.seen(),
                    since,
                    window,
                    observations,
                    owner_timeline,
                    split,
                }
                .into());
            }
            #[cfg(test)]
            readiness_poll_hook::missed(polls);
            tokio::time::sleep(READINESS_POLL_INTERVAL).await;
        }
    }
    .await;
    // Test support: a starter that collects its owner's stages sends that
    // owner's stderr to a private file, so a failed start hands it on to the
    // fixture's owner diagnostic as well.
    #[cfg(feature = "test-support")]
    if outcome.is_err()
        && let Some(diagnostic) = startup_diagnostic.as_mut()
        && let Ok(Some(mut forward)) = fixture_owner_diagnostic()
    {
        forward_startup_diagnostic(diagnostic, &mut forward);
    }
    outcome
}

/// Test-only observation of the readiness loop: a test scopes a callback
/// that runs after each failed poll, before that poll's sleep.
#[cfg(test)]
mod readiness_poll_hook {
    use std::cell::RefCell;

    pub(super) type Hook = RefCell<Box<dyn FnMut(u32)>>;

    tokio::task_local! {
        pub(super) static MISSED: Hook;
    }

    pub(super) fn missed(polls: u32) {
        let _ = MISSED.try_with(|hook| (hook.borrow_mut())(polls));
    }
}

/// A read-only inspection may attach to a published owner without electing or
/// starting one. The fallback remains an explicitly local read-only open.
///
/// The owner lock is probed only while this inspection holds the start lock:
/// no starter can then be between its election and its owner's single
/// owner-lock acquisition, which the probe could otherwise make fail.
pub(crate) async fn attach_existing_observed(
    options: &crate::store::OpenOptions,
    project: &Path,
    progress: &mut ProgressReporter,
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
        if let Some(start) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Start,
        )? {
            let owner_probe = ServiceLock::try_acquire(
                &options.data_dir,
                &options.project_scope,
                ServiceLockKind::Owner,
            );
            let owner_free = match owner_probe {
                Ok(Some(owner_probe)) => owner_probe.release().map(|()| true),
                Ok(None) => Ok(false),
                Err(error) => Err(error),
            };
            start.release()?;
            if owner_free? {
                return Ok(None);
            }
            // Another process holds owner authority. A busy start lock is an
            // election in progress instead, and is not reported as a wait.
            progress.report(MemoryOpenStage::WaitingForProjectOwnership);
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "active memory service did not publish a readable endpoint"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Why an attach attempt found no usable owner, as that attempt observed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachMiss {
    NoEndpoint,
    TransportUnavailable,
    PeerClosed,
}

impl AttachMiss {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoEndpoint => "no-endpoint",
            Self::TransportUnavailable => "transport-unavailable",
            Self::PeerClosed => "peer-closed",
        }
    }
}

/// Inspect an already-published owner only. Unlike `attach_existing`, this
/// path never probes the owner election lock or waits for a publication.
pub(crate) async fn inspect_existing(
    options: &crate::store::OpenOptions,
    project: &Path,
) -> Result<Option<ServiceAttachment>> {
    ensure!(
        options.read_only,
        "memory inspection requires read-only options"
    );
    ensure_project_scope(project, &options.project_scope)?;
    options.config.validate()?;
    try_attach(&options.data_dir, &options.project_scope, project).await
}

async fn try_attach(data: &Path, scope: &str, project: &Path) -> Result<Option<ServiceAttachment>> {
    Ok(try_attach_observed(data, scope, project, None).await?.ok())
}

/// [`try_attach`] that also names which existing check found no usable
/// owner. It performs exactly the same reads, connects and handshake. Only a
/// starter attaching to the child it just spawned presents its token.
async fn try_attach_observed(
    data: &Path,
    scope: &str,
    project: &Path,
    starter_token: Option<uuid::Uuid>,
) -> Result<std::result::Result<ServiceAttachment, AttachMiss>> {
    let Some(record) = EndpointRecord::read(data, scope)? else {
        return Ok(Err(AttachMiss::NoEndpoint));
    };
    ensure!(
        record.authority.project_path == project_path_bytes(project),
        "memory service endpoint belongs to another project path"
    );
    let mut stream = match connect_local(data, scope, &record.address, HANDSHAKE_TIMEOUT).await {
        Ok(stream) => stream,
        Err(error) => match connect_miss(&error) {
            Some(miss) => return Ok(Err(miss)),
            None => return Err(error),
        },
    };
    let identity = AttachmentIdentity::new();
    match connect_handshake_identified(
        &mut stream,
        &record.authority,
        starter_token,
        Some(identity.client),
    )
    .await
    {
        Ok(()) => {}
        // Endpoint retirement and final transport close are distinct steps.
        // A client can connect to the retiring endpoint just before the owner
        // closes it; let the existing election loop re-read discovery and
        // owner authority instead of treating that closed peer as a protocol
        // failure. Decoded rejection replies remain fatal below.
        Err(error) if is_peer_closed(&error) => return Ok(Err(AttachMiss::PeerClosed)),
        Err(error) => return Err(error),
    }
    Ok(Ok(ServiceAttachment {
        stream: Some(stream),
        held: None,
        retain_after_abandon: false,
        identity,
        authority: record.authority,
        locator: Some(AttachmentLocator {
            data: data.to_owned(),
            address: record.address,
        }),
        last_fault: None,
        #[cfg(any(test, feature = "test-support"))]
        reply_pause: None,
    }))
}

/// Whether a failed connect to a published endpoint found no usable owner,
/// and which miss it was; `None` is a fault the caller reports.
///
/// A listener that closes while this connect is still queued in its backlog
/// resets it: Linux delivers that reset from the connect itself, while Darwin
/// completes the connect and the handshake then meets the closed peer, as an
/// EOF, EPIPE or, when the close races its write, ENOTCONN. Either
/// way it is the same peer-closed observation the handshake already maps, and
/// never authority. The owner lock decides: a retiring owner reaps Dolt and
/// releases it, while a live owner that keeps closing connections keeps it
/// and is reported at the caller's existing deadline.
fn connect_miss(error: &anyhow::Error) -> Option<AttachMiss> {
    if is_transport_unavailable(error) {
        Some(AttachMiss::TransportUnavailable)
    } else if is_peer_closed(error) {
        Some(AttachMiss::PeerClosed)
    } else {
        None
    }
}

/// A published endpoint that cannot be reached is a transport observation
/// for an electing client, never owner authority: the owner may have closed
/// its listener while retiring. On Windows a pipe whose instances stayed busy
/// through the connect deadline is the same observation. A session bound to
/// one generation does not use this; it cannot elect.
fn is_transport_unavailable(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|cause| {
            matches!(
                cause.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) || is_no_free_instance(cause)
        })
}

/// Whether a Windows connect exhausted its deadline on busy pipe instances.
/// The platform connector marks that case with a typed payload; the error
/// text alone never matches.
#[cfg(windows)]
fn is_no_free_instance(error: &io::Error) -> bool {
    kuru_platform::windows::pipe::is_no_free_instance(error)
}

/// A Unix connect has no busy-instance state.
#[cfg(not(windows))]
fn is_no_free_instance(_error: &io::Error) -> bool {
    false
}

fn ensure_project_scope(project: &Path, scope: &str) -> Result<()> {
    let expected = canonical_project_scope(project)?;
    ensure!(
        scope == expected,
        "memory service project scope does not match its canonical path"
    );
    Ok(())
}

pub(crate) fn canonical_project_scope(project: &Path) -> Result<String> {
    ensure!(
        project.is_absolute() && std::fs::canonicalize(project)? == project && project.is_dir(),
        "memory service project path must be a canonical directory"
    );
    let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
    Ok(format!(
        "project/{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

/// [`attach_or_start_observed`] with nobody observing.
pub async fn attach_or_start(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
) -> Result<ServiceAttachment> {
    attach_or_start_observed(
        options,
        project,
        executable,
        &mut ProgressReporter::silent(),
    )
    .await
}

/// Reattach only to this client's original store. The elected owner carries
/// the expectation into its locked storage open; an absent store is not made.
pub(crate) async fn attach_or_start_existing(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    expected_instance: &str,
) -> Result<ServiceAttachment> {
    let mut options = options.clone();
    options.expected_instance = Some(expected_instance.to_owned());
    let attachment = attach_or_start(&options, project, executable).await?;
    ensure!(
        attachment.store_instance() == expected_instance,
        "memory store identity changed before existing-client attachment"
    );
    Ok(attachment)
}

/// [`attach_existing_observed`] with nobody observing; only tests inspect
/// without a reader.
#[cfg(test)]
pub(crate) async fn attach_existing(
    options: &crate::store::OpenOptions,
    project: &Path,
) -> Result<Option<ServiceAttachment>> {
    attach_existing_observed(options, project, &mut ProgressReporter::silent()).await
}

/// The owner's arguments. A starter token, when given, is the optional
/// tenth argument; the owner then retires on its own only after an
/// attachment presents it. Absent expected-instance and starter values use
/// explicit dashes so the final optional idle interval is unambiguous.
fn service_arguments(
    options: &crate::store::OpenOptions,
    project: &Path,
    starter_token: Option<uuid::Uuid>,
) -> Vec<OsString> {
    fn path_or_dash(path: Option<&PathBuf>) -> OsString {
        path.map_or_else(|| OsString::from("-"), |path| path.as_os_str().to_owned())
    }
    let mut arguments = vec![
        "--internal-memory-service".into(),
        project.as_os_str().to_owned(),
        options.data_dir.as_os_str().to_owned(),
        options.project_scope.clone().into(),
        path_or_dash(options.config.dolt_binary.as_ref()),
        path_or_dash(options.config.cache_dir.as_ref()),
        path_or_dash(options.supervisor.as_ref()),
        if options.config.offline { "1" } else { "0" }.into(),
        options.config.startup_timeout_secs.to_string().into(),
    ];
    arguments.push(starter_token.map_or_else(|| "-".into(), |token| token.to_string().into()));
    arguments.push(
        options
            .expected_instance
            .as_ref()
            .map_or_else(|| "-".into(), Into::into),
    );
    arguments.push(options.config.service_idle_timeout_secs.to_string().into());
    arguments
}

#[cfg(unix)]
async fn spawn_service(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    stderr: Option<File>,
    starter_token: Option<uuid::Uuid>,
) -> Result<std::process::Child> {
    let mut command = std::process::Command::new(executable);
    command.args(service_arguments(options, project, starter_token));
    command.current_dir(project);
    // Test-support measurement only: name the originating test in the trace.
    #[cfg(any(test, feature = "test-support"))]
    for (name, value) in crate::test_support::lifecycle_trace::forwarded() {
        command.env(name, value);
    }
    // Test-support only: the owner's own spawns name the same test.
    #[cfg(any(test, feature = "test-support"))]
    for (name, value) in crate::test_support::spawn_ledger::forwarded() {
        command.env(name, value);
    }
    #[cfg(test)]
    for (name, value) in activity::owner_test_environment() {
        command.env(name, value);
    }
    // Test builds hold the spawn gate across child creation; see
    // `crate::spawn_gate`.
    #[cfg(test)]
    let _creation = crate::spawn_gate::child_creation().await;
    let child = kuru_platform::unix::spawn_independent(command, stderr)
        .context("start project memory service")?;
    // Test-support only: the owner leaves this process group, so a coverage
    // partition names the test behind it if it outlives that test.
    #[cfg(any(test, feature = "test-support"))]
    crate::test_support::spawn_ledger::record(
        child.id(),
        executable,
        crate::test_support::spawn_ledger::MEMORY_OWNER,
    );
    Ok(child)
}

#[cfg(windows)]
async fn spawn_service(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    stderr: Option<File>,
    starter_token: Option<uuid::Uuid>,
) -> Result<kuru_platform::windows::process::NativeChild> {
    use kuru_platform::windows::process::{Console, Lifetime, NativeSpawnSpec, Stdio};
    let mut command = NativeSpawnSpec::new(executable.to_owned(), project.to_owned());
    command.args = service_arguments(options, project, starter_token);
    command.lifetime = Lifetime::IndependentService;
    command.console = Console::PrivateHidden;
    command.stderr = stderr.map_or(Stdio::Null, |file| Stdio::Handle(file.into()));
    let system = kuru_platform::windows::process::system_directory()?;
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(unused_mut, reason = "only test support adds layers")
    )]
    let mut layers: Vec<Vec<(OsString, OsString)>> = Vec::new();
    #[cfg(feature = "test-support")]
    if fixture_startup_stages_enabled() {
        layers.push(vec![(
            STARTUP_STAGE_DIAGNOSTIC_ENV.into(),
            OsString::from("1"),
        )]);
    }
    #[cfg(any(test, feature = "test-support"))]
    layers.push(activity::forwarded_test_hooks());
    // Test-support measurement only: forward the inert-by-default trace.
    #[cfg(any(test, feature = "test-support"))]
    layers.push(crate::test_support::lifecycle_trace::forwarded());
    // Test-support only: the owner's own spawns name the same test.
    #[cfg(any(test, feature = "test-support"))]
    layers.push(crate::test_support::spawn_ledger::forwarded());
    #[cfg(test)]
    layers.push(activity::owner_test_environment());
    command.environment = owner_environment(
        system,
        std::env::var_os("LLVM_PROFILE_FILE"),
        open_timeline::gate_set(),
        layers,
    )
    .context("compose the memory service environment")?;
    let child = command
        .spawn()
        .await
        .context("start independent or outer-contained project memory service")?;
    // Test-support only: name the test behind an owner that outlives it.
    #[cfg(any(test, feature = "test-support"))]
    crate::test_support::spawn_ledger::record(
        child.id(),
        executable,
        crate::test_support::spawn_ledger::MEMORY_OWNER,
    );
    Ok(child)
}

/// The Windows owner's explicit environment: the system variables, the
/// coverage destination when set, and the timeline gate only when this
/// starter's own gate is exactly `1`. Each later layer overrides the result
/// so far through `merge_environment`, one layer at a time, so a key that
/// arrives from two layers (a test's `KURU_OPEN_TIMELINE=0`, or a hook set
/// both in the runner and a test's own scope) replaces its case-equivalent
/// entry instead of becoming a duplicate the spawn would refuse.
#[cfg(windows)]
fn owner_environment(
    system: PathBuf,
    profile: Option<OsString>,
    gate: bool,
    layers: Vec<Vec<(OsString, OsString)>>,
) -> io::Result<Vec<(OsString, OsString)>> {
    let windows = system
        .parent()
        .ok_or_else(|| io::Error::other("Windows system directory has no parent"))?
        .as_os_str()
        .to_owned();
    let mut base = vec![
        ("SystemRoot".into(), windows.clone()),
        ("WINDIR".into(), windows),
        ("PATH".into(), system.into_os_string()),
    ];
    if let Some(profile) = profile {
        base.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    if gate {
        base.push((open_timeline::ENV.into(), OsString::from("1")));
    }
    layers
        .into_iter()
        .try_fold(base, kuru_platform::windows::process::merge_environment)
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

/// Test-only retained actual owner and the attachment of the fixture that
/// started it. Its stderr is a caller-owned private file; ordinary service
/// launches continue to discard stderr. The fixture presented its starter
/// token, so the owner retires on its own once this attachment and every
/// other client have released.
#[cfg(feature = "test-support")]
pub struct FixtureLoggedOwner {
    process: ServiceProcess,
    attachment: ServiceAttachment,
}

#[cfg(feature = "test-support")]
impl FixtureLoggedOwner {
    /// The starter's own attachment. The owner counts it like any client,
    /// so a request sent on it is served while it is the sole attachment.
    pub fn attachment(&mut self) -> &mut ServiceAttachment {
        &mut self.attachment
    }

    /// Release the starter's attachment, then await the owner's own exit.
    pub async fn wait_for_exit(mut self) -> Result<()> {
        self.attachment.close();
        // Its last attachment gone, the owner closes its store and exits; the
        // product bounds that close by `server::close_budget()`.
        let deadline = tokio::time::Instant::now() + crate::server::close_budget();
        loop {
            if let Some(status) = self.process.try_wait()? {
                self.process.0.take();
                ensure!(
                    status.success(),
                    "fixture memory owner exited unsuccessfully"
                );
                return Ok(());
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "fixture memory owner did not exit after retirement"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Release the starter's attachment, then await the owner process's own
    /// exit within `deadline`, without polling. Everything the owner does in
    /// its close, including writing an open timeline after its lock
    /// release, precedes that exit.
    #[cfg(unix)]
    pub async fn exited(mut self, deadline: Duration) -> Result<()> {
        self.attachment.close();
        let mut child = self
            .process
            .0
            .take()
            .context("fixture memory owner was already observed")?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // Detached, as in `ServiceProcess::drop`: an owner that never exits
        // fails the deadline without holding up the runtime's shutdown.
        std::thread::spawn(move || {
            let _ = sender.send(child.wait());
        });
        let status = tokio::time::timeout(deadline, receiver)
            .await
            .context("fixture memory owner did not exit after retirement")?
            .context("fixture memory owner exit waiter ended")??;
        ensure!(
            status.success(),
            "fixture memory owner exited unsuccessfully"
        );
        Ok(())
    }
}

#[cfg(feature = "test-support")]
pub(crate) async fn spawn_logged_owner_fixture(
    options: &crate::store::OpenOptions,
    project: &Path,
    executable: &Path,
    diagnostic: File,
) -> Result<FixtureLoggedOwner> {
    ensure_project_scope(project, &options.project_scope)?;
    ensure!(
        executable.is_absolute(),
        "fixture executable must be absolute"
    );
    ensure!(
        try_attach(&options.data_dir, &options.project_scope, project)
            .await?
            .is_none(),
        "fixture already has a managed memory owner"
    );
    let starter_token = uuid::Uuid::new_v4();
    let mut child = ServiceProcess::new(
        spawn_service(
            options,
            project,
            executable,
            Some(diagnostic),
            Some(starter_token),
        )
        .await?,
    );
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(options.config.startup_timeout_secs);
    loop {
        if let Ok(attachment) = try_attach_observed(
            &options.data_dir,
            &options.project_scope,
            project,
            Some(starter_token),
        )
        .await?
        {
            return Ok(FixtureLoggedOwner {
                process: child,
                attachment,
            });
        }
        ensure!(
            child.try_wait()?.is_none(),
            "fixture memory owner exited before readiness"
        );
        ensure!(
            tokio::time::Instant::now() < deadline,
            "fixture memory owner did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub struct ServiceListener {
    #[cfg(unix)]
    inner: kuru_platform::local_ipc::PrivateServiceListener,
    #[cfg(windows)]
    inner: kuru_platform::windows::pipe::PrivateServiceListener,
}

/// Which authenticated attachment marks a new owner as reached by the client
/// that started it. Until then an empty owner waits for that client instead
/// of retiring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Only the attachment that presents this starter token.
    Starter(uuid::Uuid),
    /// Any authenticated attachment: an owner started without a token, by a
    /// starter from before the token existed.
    #[default]
    AnyAttachment,
    /// No attachment: an in-process test owner, which ends only through
    /// maintenance retirement, a fixture restart or lock loss.
    #[cfg(test)]
    Never,
}

/// Serve-loop policy. [`ServiceOwner::serve`] derives it from the owner's
/// starter token; in-process tests choose their own through `serve_with`.
pub(crate) struct ServeKnobs {
    pub(crate) admission: Admission,
    /// How long after endpoint publication an owner not yet reached keeps
    /// waiting for its starter once empty. `None`: without bound.
    pub(crate) first_attachment: Option<Duration>,
    /// Longest single accept wait before the owner lock is re-verified.
    pub(crate) recheck: Duration,
    /// Retention after the last attachment task joins; zero retires immediately.
    pub(crate) idle: Duration,
    #[cfg(test)]
    pub(crate) observer: Option<tokio::sync::mpsc::UnboundedSender<ServeEvent>>,
    #[cfg(test)]
    pub(crate) close_pause: Option<Arc<ClosePause>>,
    #[cfg(test)]
    pub(crate) dispatch_pause: Option<Arc<rpc::DispatchPause>>,
    /// Test-only: every accept fails at once with this text, as a
    /// non-timeout listener fault does.
    #[cfg(test)]
    pub(crate) accept_fault: Option<&'static str>,
    /// A test's own open timeline, written at close in place of the process
    /// timeline, which a test runner never installs.
    #[cfg(test)]
    pub(crate) timeline: Option<Arc<crate::open_timeline::Timeline>>,
}

#[cfg(test)]
impl ServeKnobs {
    /// An in-process fixture owner: never reached, so it outlives its last
    /// client until maintenance retires it, as fixtures expect.
    pub(crate) fn never_reached() -> Self {
        Self {
            admission: Admission::Never,
            first_attachment: None,
            recheck: OWNER_LOCK_RECHECK_INTERVAL,
            idle: Duration::ZERO,
            observer: None,
            close_pause: None,
            dispatch_pause: None,
            accept_fault: None,
            timeline: None,
        }
    }

    fn emit(&self, event: ServeEvent) {
        if let Some(observer) = &self.observer {
            let _ = observer.send(event);
        }
    }
}

/// Serve-loop events, in the order the loop acts on them. `EnteredEmpty` is
/// sent when the loop first finds no attachment: at the start and after the
/// join that ended the last one, not again on each lock recheck.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServeEvent {
    AttachmentAccepted { active: usize },
    AttachmentJoined { remaining: usize },
    EnteredEmpty { reached: bool },
    LockRechecked,
}

/// Where a test holds an owner's shutdown, in shutdown order.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosePoint {
    BeforeListenerDrop,
    AfterListenerDrop,
    AfterEndpointRetire,
    /// Immediately before the store close: after the activity record's
    /// retirement or failing mark.
    BeforeStoreClose,
    /// After the store close and Dolt reap, before the owner lock release.
    AfterReap,
    /// After the owner lock release, before the gated open timeline write.
    AfterRelease,
}

/// An owner-local test barrier at each of its [`ClosePoint`]s. Modelled on
/// `ReplyPause`: at each point the owner notifies `entered`, then waits for
/// `release`.
#[cfg(test)]
pub(crate) struct ClosePause {
    at: Vec<ClosePoint>,
    pub(crate) entered: tokio::sync::Notify,
    pub(crate) release: tokio::sync::Notify,
}

#[cfg(test)]
impl ClosePause {
    pub(crate) fn at(point: ClosePoint) -> Arc<Self> {
        Self::at_each(&[point])
    }

    pub(crate) fn at_each(points: &[ClosePoint]) -> Arc<Self> {
        Arc::new(Self {
            at: points.to_vec(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }

    async fn reached(pause: Option<&Self>, point: ClosePoint) {
        if let Some(pause) = pause.filter(|pause| pause.at.contains(&point)) {
            pause.entered.notify_one();
            pause.release.notified().await;
        }
    }
}

/// The service process owns this state after the short starter election lock
/// has been released. The caller must stop accepting and drain live handlers
/// before `close`; normal shutdown retires discovery, then reaps Dolt before
/// releasing owner authority.
pub struct ServiceOwner {
    lock: ServiceLock,
    store: crate::store::MemoryStore,
    listener: ServiceListener,
    record: EndpointRecord,
    data_dir: PathBuf,
    receipt_progress: std::sync::Arc<rpc::ReceiptProgress>,
    starter_token: Option<uuid::Uuid>,
    /// The finished publisher of this owner's open-activity record, retired
    /// inside `close` while owner authority is still held.
    activity: Option<activity::Publisher>,
    startup_timeout: Duration,
    idle_timeout: Duration,
    /// Taken after the endpoint record was published.
    published: tokio::time::Instant,
}

impl ServiceOwner {
    pub async fn open(options: crate::store::OpenOptions, project_path: &Path) -> Result<Self> {
        Self::open_hooked(options, project_path, activity::OwnerHooks::from_env()).await
    }

    /// [`Self::open`] with a test's own activity hooks instead of the
    /// process environment.
    #[cfg(test)]
    pub(crate) async fn open_with_activity(
        options: crate::store::OpenOptions,
        project_path: &Path,
        hooks: activity::OwnerHooks,
    ) -> Result<Self> {
        Self::open_hooked(options, project_path, hooks).await
    }

    async fn open_hooked(
        options: crate::store::OpenOptions,
        project_path: &Path,
        hooks: activity::OwnerHooks,
    ) -> Result<Self> {
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
        // A gated owner's stream, named from its starter's token, exists
        // only while this owner lock is held: it is removed before serving
        // and on every failed open below, never in close.
        if open_timeline::installed().is_some()
            && let Some(token) = &options.starter_token
            && let Ok(directory) =
                EndpointRecord::directory(&options.data_dir, &options.project_scope)
        {
            open_timeline::stream_to(&directory, &activity::activity_tag(token));
        }
        open_timeline::stamp(open_timeline::Event::OwnerLock);
        #[cfg(test)]
        if let Some(slot) = &hooks.duplicate_owner_lock {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(lock.file().try_clone()?);
        }
        #[cfg(test)]
        let end_pause = hooks.close_pause.clone();
        // On failure the store open retires its own record before returning,
        // while this owner lock is still held.
        let (store, activity) = activity::open_owner_store(options.clone(), hooks)
            .await
            .inspect_err(|_| open_timeline::end_stream())?;
        // Kept out of the failing mark of a publication that fails.
        let mut connection_secret = None;
        let prepared = async {
            let (listener, address) =
                ServiceListener::bind(&options.data_dir, &options.project_scope)?;
            open_timeline::stamp(open_timeline::Event::ListenerBound);
            let record =
                EndpointRecord::for_store(project_path, &options.project_scope, &store, address)
                    .await?;
            connection_secret = Some(record.authority.connection_secret.clone());
            record.publish(&options.data_dir, &lock)?;
            open_timeline::stamp(open_timeline::Event::EndpointPublished);
            // Before serving and before this open returns.
            open_timeline::end_stream();
            Ok::<_, anyhow::Error>((listener, record))
        }
        .await;
        let (listener, record) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                open_timeline::end_stream();
                // A failed listener or publication must still reap Dolt while
                // this process retains its service-owner authority. This
                // owner will not serve its starter, so its record carries the
                // failure.
                let closed = close_store_and_record(
                    store,
                    activity,
                    &lock,
                    &options.data_dir,
                    &options.project_scope,
                    Some(&error),
                    connection_secret.as_deref(),
                    #[cfg(test)]
                    end_pause.as_deref(),
                )
                .await;
                // Released explicitly after the reap and before this open
                // returns: a starter elects a successor on this lock.
                let released = lock.release();
                let error = match closed {
                    Err(cleanup) => error.context(format!(
                        "reap Dolt after memory service startup failed: {cleanup:#}"
                    )),
                    Ok(()) => error,
                };
                return Err(match released {
                    Err(release) => error.context(format!(
                        "release memory service owner authority after startup failed: {release:#}"
                    )),
                    Ok(()) => error,
                });
            }
        };
        Ok(Self {
            lock,
            store,
            listener,
            record,
            data_dir: options.data_dir,
            receipt_progress: std::sync::Arc::new(rpc::ReceiptProgress::default()),
            starter_token: options.starter_token,
            activity,
            startup_timeout: Duration::from_secs(options.config.startup_timeout_secs),
            idle_timeout: Duration::from_secs(options.config.service_idle_timeout_secs),
            published: tokio::time::Instant::now(),
        })
    }

    pub fn authority(&self) -> &EndpointAuthority {
        &self.record.authority
    }

    #[cfg(test)]
    pub(crate) fn inspection_store_for_test(&self) -> crate::store::MemoryStore {
        self.store.clone()
    }

    pub async fn accept(&mut self, deadline: Duration) -> Result<LocalStream> {
        self.lock.verify()?;
        self.listener.accept(deadline).await
    }

    /// The product policy: reached by the attachment that presents this
    /// owner's starter token, or by any attachment when started without one.
    pub(crate) fn knobs(&self) -> ServeKnobs {
        ServeKnobs {
            admission: self
                .starter_token
                .map_or(Admission::AnyAttachment, Admission::Starter),
            first_attachment: Some(self.startup_timeout),
            recheck: OWNER_LOCK_RECHECK_INTERVAL,
            idle: self.idle_timeout,
            #[cfg(test)]
            observer: None,
            #[cfg(test)]
            close_pause: None,
            #[cfg(test)]
            dispatch_pause: None,
            #[cfg(test)]
            accept_fault: None,
            #[cfg(test)]
            timeline: None,
        }
    }

    /// Run until the bounded idle interval after the last attachment settles.
    /// Each connection has one generation-bound attachment; the
    /// store itself keeps reads concurrent and serializes short writes.
    pub async fn serve(self) -> Result<()> {
        let knobs = self.knobs();
        self.serve_knobs(knobs).await
    }

    /// [`Self::serve`] under a test's own policy, observer and pauses.
    #[cfg(test)]
    pub(crate) async fn serve_with(self, knobs: ServeKnobs) -> Result<()> {
        self.serve_knobs(knobs).await
    }

    async fn serve_knobs(mut self, knobs: ServeKnobs) -> Result<()> {
        let (served, reached) = self.serve_until_retired(&knobs).await;
        // An owner whose loop failed before its starter attached will not
        // serve that starter: its record carries the failure.
        let unserved = served.as_ref().err().filter(|_| !reached);
        #[cfg(test)]
        let closed = self
            .close_paused(
                unserved,
                knobs.close_pause.as_deref(),
                knobs.timeline.as_deref(),
            )
            .await;
        #[cfg(not(test))]
        let closed = self.close_paused(unserved).await;
        served.and(closed)
    }

    /// Serve until retirement, and say whether the starter had attached.
    async fn serve_until_retired(&mut self, knobs: &ServeKnobs) -> (Result<()>, bool) {
        let retirement = std::sync::Arc::new(rpc::Retirement::new(knobs.admission));
        let served = self.serve_attachments(knobs, &retirement).await;
        // Every attachment task was joined, or aborted and drained, before
        // the loop returned, which orders its `reached` write before this read.
        (served, retirement.reached())
    }

    async fn serve_attachments(
        &mut self,
        knobs: &ServeKnobs,
        retirement: &std::sync::Arc<rpc::Retirement>,
    ) -> Result<()> {
        let mut attachments = tokio::task::JoinSet::new();
        let frame_budget = std::sync::Arc::new(tokio::sync::Semaphore::new(rpc::FRAME_BUDGET_MIB));
        #[cfg(test)]
        if let Some(pause) = &knobs.dispatch_pause {
            retirement.pause_next_dispatch(pause.clone());
        }
        // Absolute, so rejected or non-starter connections never restart it.
        let starter_deadline = knobs.first_attachment.map(|within| self.published + within);
        let mut idle_deadline = None;
        #[cfg(test)]
        let mut was_empty = false;
        loop {
            if let Err(error) = self.lock.verify() {
                abort_and_drain(&mut attachments).await;
                return Err(error);
            }
            #[cfg(test)]
            knobs.emit(ServeEvent::LockRechecked);
            if retirement.requested() {
                while let Some(completed) = attachments.join_next().await {
                    if let Err(error) = completed {
                        tracing::warn!(error = %error, "memory service retirement attachment failed");
                    }
                }
                break;
            }
            if attachments.is_empty() {
                // `reached` is set inside an attachment task; the join that
                // emptied the set orders that write before this read.
                let reached = retirement.reached();
                #[cfg(test)]
                if !std::mem::replace(&mut was_empty, true) {
                    knobs.emit(ServeEvent::EnteredEmpty { reached });
                }
                // Start only after every handler joins. Rechecks do not extend
                // the interval; an attachment begins a fresh busy period.
                let deadline = if reached {
                    Some(
                        *idle_deadline
                            .get_or_insert_with(|| tokio::time::Instant::now() + knobs.idle),
                    )
                } else {
                    starter_deadline
                };
                let accept_within = match deadline {
                    Some(deadline) => {
                        let remaining =
                            deadline.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() {
                            if !reached {
                                tracing::warn!(
                                    "memory service retiring: no starter attached within the startup timeout"
                                );
                            }
                            break;
                        }
                        remaining.min(knobs.recheck)
                    }
                    None => knobs.recheck,
                };
                tokio::select! {
                    biased;
                    accepted = self.accept_once(knobs, accept_within) => match accepted {
                        Ok(stream) => {
                            if self.attach(stream, &mut attachments, &frame_budget, retirement) {
                                #[cfg(test)]
                                {
                                    was_empty = false;
                                    knobs.emit(ServeEvent::AttachmentAccepted {
                                        active: retirement.active(),
                                    });
                                }
                            }
                        }
                        Err(error) if is_accept_timeout(&error) => continue,
                        Err(error) => return Err(error),
                    },
                    () = retirement.notified() => continue,
                }
            } else {
                idle_deadline = None;
                tokio::select! {
                    accepted = self.accept_once(knobs, knobs.recheck) => {
                        match accepted {
                            Ok(stream) => {
                                if self.attach(stream, &mut attachments, &frame_budget, retirement) {
                                    #[cfg(test)]
                                    knobs.emit(ServeEvent::AttachmentAccepted {
                                        active: retirement.active(),
                                    });
                                }
                            }
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
                        #[cfg(test)]
                        knobs.emit(ServeEvent::AttachmentJoined {
                            remaining: attachments.len(),
                        });
                    }
                    () = retirement.notified() => continue,
                }
            }
        }
        Ok(())
    }

    /// One listener accept, or a test's injected listener fault.
    async fn accept_once(&mut self, knobs: &ServeKnobs, within: Duration) -> Result<LocalStream> {
        #[cfg(test)]
        if let Some(fault) = knobs.accept_fault {
            anyhow::bail!(fault);
        }
        #[cfg(not(test))]
        let _ = knobs;
        self.listener.accept(within).await
    }

    fn attach(
        &self,
        stream: LocalStream,
        attachments: &mut tokio::task::JoinSet<()>,
        frame_budget: &std::sync::Arc<tokio::sync::Semaphore>,
        retirement: &std::sync::Arc<rpc::Retirement>,
    ) -> bool {
        if attachments.len() >= MAX_ATTACHMENTS {
            return false;
        }
        let Some(retained) = retirement.attached() else {
            return false;
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
        true
    }

    /// Close only after accepted requests and client attachments have
    /// drained: stop accepting, await the activity publisher's last write,
    /// retire the endpoint, close the store (its
    /// write drain and the Dolt reap, which ends the lifecycle lease), and
    /// only then release owner authority. A crash instead leaves a stale
    /// record which a successor reconciles only after obtaining owner and
    /// existing lifecycle authority.
    pub async fn close(self) -> Result<()> {
        #[cfg(test)]
        let closed = self.close_paused(None, None, None).await;
        #[cfg(not(test))]
        let closed = self.close_paused(None).await;
        closed
    }

    /// The close, given the failure that ended an owner before its starter
    /// attached, if one did.
    async fn close_paused(
        self,
        unserved: Option<&anyhow::Error>,
        #[cfg(test)] pause: Option<&ClosePause>,
        #[cfg(test)] timeline: Option<&crate::open_timeline::Timeline>,
    ) -> Result<()> {
        let Self {
            lock,
            store,
            listener,
            record,
            data_dir,
            mut activity,
            ..
        } = self;
        #[cfg(test)]
        ClosePause::reached(pause, ClosePoint::BeforeListenerDrop).await;
        drop(listener);
        #[cfg(test)]
        ClosePause::reached(pause, ClosePoint::AfterListenerDrop).await;
        // The publisher's last write follows its open on its own task, and
        // nothing before here waited for it. Awaiting it now, as retirement
        // already would, means that once the endpoint is gone the record
        // changes only by its failing mark or its retirement: a starter whose
        // attach then fails reads a settled record, never a late replacement.
        if let Some(publisher) = activity.as_mut() {
            publisher.finish_writes().await;
        }
        let retired = record.retire(&data_dir, &lock);
        #[cfg(test)]
        ClosePause::reached(pause, ClosePoint::AfterEndpointRetire).await;
        // A failed retirement still closes the store and reaps Dolt; the
        // owner lock is released only after that close has returned.
        let closed = close_store_and_record(
            store,
            activity,
            &lock,
            &data_dir,
            &record.authority.project_scope,
            unserved,
            Some(&record.authority.connection_secret),
            #[cfg(test)]
            pause,
        )
        .await;
        #[cfg(test)]
        ClosePause::reached(pause, ClosePoint::AfterReap).await;
        let released = lock.release();
        #[cfg(test)]
        ClosePause::reached(pause, ClosePoint::AfterRelease).await;
        // Diagnostics only, after the release a successor's wait ends at: the
        // name is this generation's own, and nothing here can fail or reorder
        // this close or a successor's open. A gated close adds only this one
        // create-only, unsynced write of a bounded size before it returns.
        #[cfg(test)]
        let timeline = timeline.or(open_timeline::installed());
        #[cfg(not(test))]
        let timeline = open_timeline::installed();
        if let Some(timeline) = timeline
            && let Ok(directory) =
                EndpointRecord::directory(&data_dir, &record.authority.project_scope)
        {
            let _ =
                open_timeline::write(timeline, &directory, &record.authority.service_generation);
        }
        retired.and(closed).and(released)
    }
}

/// Close an owner's store and retire its activity record, in one order for
/// both of its endings. Still under owner authority, so no successor's record
/// can share the name yet; best effort: no failure of the record's mark or
/// retirement fails the close.
///
/// An owner whose store opened but which ends before its starter attached
/// (`unserved`) marks its record failing with that error's own text, the
/// store's identity secrets and `connection_secret` redacted, and awaits that
/// write as retirement already awaits the publisher. It then closes its store
/// and reaps Dolt, and only then retires the record, so its starter reads
/// the reason at once instead of a retirement that carries none. The mark is
/// written only under verified owner authority, since the write replaces
/// whatever record holds the name; without it, or when the write fails, and
/// for every other ending, the record is retired before the store closes.
/// The record is retired whatever the close returns.
#[allow(
    clippy::too_many_arguments,
    reason = "one ending order for both owner endings; the test pause is cfg(test)"
)]
async fn close_store_and_record(
    store: crate::store::MemoryStore,
    mut activity: Option<activity::Publisher>,
    lock: &ServiceLock,
    data_dir: &Path,
    scope: &str,
    unserved: Option<&anyhow::Error>,
    connection_secret: Option<&str>,
    #[cfg(test)] pause: Option<&ClosePause>,
) -> Result<()> {
    let mut marked = false;
    if let (Some(publisher), Some(error)) = (activity.as_mut(), unserved)
        && lock.verify().is_ok()
    {
        let reason = connection_secret
            .filter(|secret| !secret.is_empty())
            .into_iter()
            .fold(store.failure_reason(error), |reason, secret| {
                reason.replace(secret, "[redacted]")
            });
        marked = publisher.mark_failing(&reason).await;
    }
    if !marked && let Some(publisher) = activity.take() {
        activity::retire(publisher, data_dir, scope).await;
    }
    #[cfg(test)]
    ClosePause::reached(pause, ClosePoint::BeforeStoreClose).await;
    let closed = store.close().await;
    if let Some(publisher) = activity {
        activity::retire(publisher, data_dir, scope).await;
    }
    closed
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

/// A held start or owner lock. Every ending unlocks explicitly before its
/// handle closes, whether through [`Self::release`] or by drop: on Unix a
/// sibling's child between fork and exec can hold a duplicate of this lock's
/// open description, and closing only this descriptor would leave the lock
/// held for that child's lifetime instead of ending it now.
pub struct ServiceLock {
    directory: Directory,
    name: OsString,
    /// Present until the lock is released; taken only by its release.
    file: Option<File>,
    kind: ServiceLockKind,
}

impl Drop for ServiceLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            crate::files::release_lock(file);
        }
    }
}

/// Retains both election and owner authority while an explicit maintenance
/// operation inspects or moves this project's storage. An active service must
/// retire before this permit can be acquired; a new starter cannot elect until
/// the permit is dropped. Fields drop in declaration order, so the owner lock
/// is released before the start lock, the reverse of their acquisition: a
/// starter that elects on the freed start lock then finds the owner lock free.
pub(crate) struct MaintenancePermit {
    _owner: ServiceLock,
    _start: ServiceLock,
    _native: crate::session_driver::NativeMaintenanceLease,
}

/// The checked owner refused maintenance because clients remain attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MaintenanceActiveClients;

impl std::fmt::Display for MaintenanceActiveClients {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("memory service has active clients; close them before maintenance")
    }
}

impl std::error::Error for MaintenanceActiveClients {}

/// Where a maintenance permit acquisition is, for a caller whose own bound
/// may cancel it: that caller can name the step it was cancelled in instead
/// of reporting only that its deadline elapsed.
#[derive(Default)]
pub(crate) struct MaintenanceTrace {
    step: std::sync::Mutex<MaintenanceStep>,
    /// Wakes every waiter on the trace's replies after each recorded reply.
    #[cfg(any(test, feature = "test-support"))]
    replied: tokio::sync::Notify,
    /// Reads the owner's records at a reply that found no live endpoint and
    /// says whether the owner is still opening. An opening owner has not
    /// published yet, so that reply is not a closing reading.
    #[cfg(any(test, feature = "test-support"))]
    opening: Option<Box<dyn Fn() -> bool + Send + Sync>>,
}

/// The first retirement reply that showed the owner closing, and when it
/// arrived: no live endpoint, a connection the owner closed unanswered, or
/// an accepted retirement request.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
pub(crate) struct ClosingReading {
    pub(crate) since: tokio::time::Instant,
    pub(crate) reply: &'static str,
}

#[derive(Clone, Copy, Default)]
struct MaintenanceStep {
    phase: MaintenancePhase,
    /// When the current phase began.
    since: Option<tokio::time::Instant>,
    /// When the wait for the current lock (start, then owner) began.
    lock_since: Option<tokio::time::Instant>,
    /// Retirement requests that found no live endpoint to ask.
    unanswered: u32,
    /// Retirement requests whose connection the owner closed unanswered, at
    /// connect or handshake. A retiring owner does this once or twice; a
    /// count that keeps growing while the owner lock stays held names a live
    /// owner that is closing connections.
    peer_closed: u32,
    /// Retirement requests the owner refused because clients were attached.
    busy: u32,
    /// Requests without a live endpoint while the owner's records showed it
    /// still opening.
    #[cfg(any(test, feature = "test-support"))]
    opening: u32,
    /// The first reply that showed the owner closing.
    #[cfg(any(test, feature = "test-support"))]
    closing: Option<ClosingReading>,
}

#[derive(Clone, Copy, Default)]
enum MaintenancePhase {
    #[default]
    NotStarted,
    StartLock,
    OwnerLock,
    Requesting,
    AwaitingRetirement,
}

impl MaintenanceTrace {
    /// Begin waiting for the next lock.
    fn wait_for(&self, phase: MaintenancePhase) {
        let now = tokio::time::Instant::now();
        let mut step = self
            .step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        step.lock_since = Some(now);
        step.phase = phase;
        step.since = Some(now);
    }

    fn enter(&self, phase: MaintenancePhase) {
        let mut step = self
            .step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        step.phase = phase;
        step.since = Some(tokio::time::Instant::now());
    }

    /// A trace whose replies without a live endpoint are closing readings
    /// only while `opening` says the owner is not still opening.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn reading_opening(opening: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            opening: Some(Box::new(opening)),
            ..Self::default()
        }
    }

    fn record(&self, reply: RetirementReply) {
        // Read before the step lock: the classifier reads the owner's records.
        #[cfg(any(test, feature = "test-support"))]
        let opening = reply == RetirementReply::NoEndpoint
            && self.opening.as_ref().is_some_and(|opening| opening());
        let mut step = self
            .step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match reply {
            RetirementReply::NoEndpoint => step.unanswered = step.unanswered.saturating_add(1),
            RetirementReply::PeerClosed => step.peer_closed = step.peer_closed.saturating_add(1),
            RetirementReply::Busy => step.busy = step.busy.saturating_add(1),
            RetirementReply::Accepted => {}
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            if opening {
                step.opening = step.opening.saturating_add(1);
            }
            if step.closing.is_none() {
                let reply = match reply {
                    RetirementReply::NoEndpoint if opening => None,
                    RetirementReply::NoEndpoint => Some("no live endpoint"),
                    RetirementReply::PeerClosed => {
                        Some("the owner closed the connection unanswered")
                    }
                    RetirementReply::Accepted => Some("retirement accepted"),
                    RetirementReply::Busy => None,
                };
                step.closing = reply.map(|reply| ClosingReading {
                    since: tokio::time::Instant::now(),
                    reply,
                });
            }
            drop(step);
            self.replied.notify_waiters();
        }
    }

    /// Wait for the first reply showing the owner closing, and return it. A
    /// waiter that starts after that reply returns at once.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn closing(&self) -> ClosingReading {
        self.until(|step| step.closing).await
    }

    /// Wait until a retirement request has found no live endpoint. A waiter
    /// that starts after that reply returns at once.
    #[cfg(test)]
    pub(crate) async fn found_no_endpoint(&self) {
        self.until(|step| (step.unanswered > 0).then_some(())).await;
    }

    /// Wait until `reading` returns a value from the recorded replies. The
    /// wake-up is registered before each check, so a reply recorded between
    /// the check and the wait is not missed.
    #[cfg(any(test, feature = "test-support"))]
    async fn until<T>(&self, reading: impl Fn(&MaintenanceStep) -> Option<T>) -> T {
        loop {
            let notified = self.replied.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let found = reading(
                &self
                    .step
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            if let Some(found) = found {
                return found;
            }
            notified.await;
        }
    }
}

impl std::fmt::Display for MaintenanceTrace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let step = *self
            .step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let phase = match step.phase {
            MaintenancePhase::NotStarted => "not started",
            MaintenancePhase::StartLock => "waiting for the start lock",
            MaintenancePhase::OwnerLock => "waiting for the owner lock",
            MaintenancePhase::Requesting => {
                "asking the owner to retire (endpoint read, connect or reply)"
            }
            MaintenancePhase::AwaitingRetirement => {
                "waiting for the owner lock after the owner accepted retirement"
            }
        };
        let elapsed = |instant: Option<tokio::time::Instant>| {
            instant.map_or(0, |instant| instant.elapsed().as_millis())
        };
        write!(
            formatter,
            "maintenance {phase} for {}ms (this lock's wait {}ms); requests without a live endpoint={}; requests the owner closed unanswered={}; busy replies={}",
            elapsed(step.since),
            elapsed(step.lock_since),
            step.unanswered,
            step.peer_closed,
            step.busy
        )?;
        #[cfg(any(test, feature = "test-support"))]
        if self.opening.is_some() {
            write!(
                formatter,
                "; requests without a live endpoint while the owner was opening={}",
                step.opening
            )?;
        }
        Ok(())
    }
}

/// How long a maintenance permit acquisition waits for the start and owner
/// locks: the longer of the owner's own close budget
/// ([`crate::server::close_budget`]) and `memory.startup_timeout_secs`.
///
/// Each lock may be held by an owner or starter that is starting or by one
/// that is closing. The start lock's holder is a starter, bounded by its
/// startup, or another maintenance acquisition waiting out an owner's close
/// (or doing its maintenance, which this deadline bounds as before).
/// The owner lock's holder is an owner still opening (bounded by its
/// startup), serving (decided by its busy replies, not by this deadline) or
/// closing on its own or at a retirement request (bounded by its close
/// budget). Neither figure alone bounds both, so a maintenance caller that
/// meets an owner's close does not fail while that close is still within the
/// budget the owner itself allows, and a configured startup timeout longer
/// than the close budget still bounds the wait behind a starting owner.
pub(crate) fn maintenance_deadline(options: &crate::store::OpenOptions) -> Duration {
    crate::server::close_budget().max(Duration::from_secs(options.config.startup_timeout_secs))
}

pub(crate) async fn acquire_maintenance_permit(
    options: &crate::store::OpenOptions,
) -> Result<MaintenancePermit> {
    acquire_maintenance_permit_traced(options, &MaintenanceTrace::default()).await
}

/// [`acquire_maintenance_permit`], recording each step in `trace`.
///
/// One deadline, counted from the acquisition's start, bounds the start-lock
/// wait, the owner's reply and the owner-lock wait: [`maintenance_deadline`].
pub(crate) async fn acquire_maintenance_permit_traced(
    options: &crate::store::OpenOptions,
    trace: &MaintenanceTrace,
) -> Result<MaintenancePermit> {
    options.config.validate()?;
    ensure!(
        !options.read_only,
        "memory maintenance requires writable options"
    );
    // A vanished service is not proof that a surviving conversation's native
    // work drained. Acquire the exact existing project barrier first and keep
    // it through owner/start ownership and all maintenance directory moves.
    let native = crate::session_driver::NativeMaintenanceLease::acquire(
        &options.data_dir,
        &options.project_scope,
    )?;
    let deadline = tokio::time::Instant::now() + maintenance_deadline(options);
    trace.wait_for(MaintenancePhase::StartLock);
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
    trace.wait_for(MaintenancePhase::OwnerLock);
    let owner = loop {
        if let Some(lock) = ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            break lock;
        }
        if !retirement_requested {
            trace.enter(MaintenancePhase::Requesting);
            let reply = tokio::time::timeout_at(deadline, request_idle_retirement(options))
                .await
                .with_context(|| {
                    format!("memory maintenance owner-response deadline exceeded; {trace}")
                })??;
            trace.record(reply);
            trace.enter(if reply == RetirementReply::Accepted {
                MaintenancePhase::AwaitingRetirement
            } else {
                MaintenancePhase::OwnerLock
            });
            match reply {
                RetirementReply::Accepted => retirement_requested = true,
                RetirementReply::Busy => {
                    busy_observations += 1;
                    if busy_observations >= 10 {
                        return Err(MaintenanceActiveClients.into());
                    }
                }
                // Neither is owner authority: wait for the owner lock.
                RetirementReply::NoEndpoint | RetirementReply::PeerClosed => {}
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "memory service owner is still active; maintenance cannot proceed; {trace}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    start.verify()?;
    owner.verify()?;
    Ok(MaintenancePermit {
        _owner: owner,
        _start: start,
        _native: native,
    })
}

/// What one request for idle retirement observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetirementReply {
    /// No endpoint record, or nothing listening at the published one.
    NoEndpoint,
    /// The owner closed the connection unanswered, at connect or handshake,
    /// as a retiring owner closes one it will never accept.
    PeerClosed,
    /// The owner refused: clients are attached.
    Busy,
    Accepted,
}

/// The caller holds the start gate, so a successful idle retirement cannot
/// race a replacement election while the owner reaps Dolt. A missing/stale
/// endpoint and a connection the owner closed unanswered are wait
/// conditions decided by the owner lock; a valid owner with live clients
/// refuses.
async fn request_idle_retirement(options: &crate::store::OpenOptions) -> Result<RetirementReply> {
    let Some(record) = EndpointRecord::read(&options.data_dir, &options.project_scope)? else {
        return Ok(RetirementReply::NoEndpoint);
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
        Err(error) => match connect_miss(&error) {
            Some(AttachMiss::PeerClosed) => return Ok(RetirementReply::PeerClosed),
            Some(AttachMiss::TransportUnavailable | AttachMiss::NoEndpoint) => {
                return Ok(RetirementReply::NoEndpoint);
            }
            None => return Err(error).context("connect to memory service for maintenance"),
        },
    };
    let mut attachment = ServiceAttachment {
        stream: Some(stream),
        held: None,
        retain_after_abandon: false,
        identity: AttachmentIdentity::new(),
        authority: record.authority,
        locator: None,
        last_fault: None,
        #[cfg(any(test, feature = "test-support"))]
        reply_pause: None,
    };
    match connect_handshake(
        attachment.stream.as_mut().expect("new maintenance stream"),
        &attachment.authority,
    )
    .await
    {
        Ok(()) => {}
        // An owner retiring on its own closes a connection it will never
        // accept; wait for its owner lock as for a missing endpoint.
        Err(error) if is_peer_closed(&error) => return Ok(RetirementReply::PeerClosed),
        // Each failing write carries its own context, so a log names the
        // hello or the request. `is_peer_closed` walks the chain.
        Err(error) => return Err(error).context("memory service maintenance handshake"),
    }
    let result = attachment.call(ServiceCall::RetireIfIdle).await;
    attachment.close();
    match result.context("memory service idle-retirement request")? {
        ServiceValue::Retirement { accepted: true } => Ok(RetirementReply::Accepted),
        ServiceValue::Retirement { accepted: false } => Ok(RetirementReply::Busy),
        _ => bail!("memory service returned the wrong maintenance response"),
    }
}

impl ServiceLock {
    fn location(
        data_dir: &Path,
        scope: &str,
        kind: ServiceLockKind,
    ) -> Result<(Directory, OsString)> {
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
        Ok((directory, name))
    }

    /// A busy owner lock means an existing process may still be cleaning up.
    /// Never remove its lockfile or infer takeover authority from a PID.
    pub fn try_acquire(
        data_dir: &Path,
        scope: &str,
        kind: ServiceLockKind,
    ) -> Result<Option<Self>> {
        let (directory, name) = Self::location(data_dir, scope, kind)?;
        let file = directory.lock_file(&name)?;
        match file.try_lock() {
            Ok(()) => {
                let lock = Self {
                    directory,
                    name,
                    file: Some(file),
                    kind,
                };
                lock.verify()?;
                Ok(Some(lock))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(error).context("acquire project service lock"),
        }
    }

    fn file(&self) -> &File {
        self.file
            .as_ref()
            .expect("a service lock holds its handle until released")
    }

    pub fn verify(&self) -> Result<()> {
        self.directory.verify(&self.name, self.file())?;
        Ok(())
    }

    /// Verify, then unlock explicitly before closing. On Unix a sibling's
    /// child between fork and exec can hold a duplicate of this lock's open
    /// description; closing only this descriptor would leave it locked. A
    /// failed verification still unlocks this description, through drop.
    pub fn release(mut self) -> Result<()> {
        self.verify()?;
        let file = self
            .file
            .take()
            .expect("a service lock holds its handle until released");
        file.unlock().context("release project service lock")?;
        Ok(())
    }

    /// Test support: wait, without polling, until whoever holds this kind of
    /// lock releases it, then release it again at once. It waits on the lock
    /// itself, so a spawned owner's exit is observed through its own release
    /// of owner authority. It has no deadline of its own; the caller's
    /// fixture backstop bounds it, and a blocked waiter ends with the process.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn await_release(data_dir: &Path, scope: &str, kind: ServiceLockKind) -> Result<()> {
        let data_dir = data_dir.to_owned();
        let scope = scope.to_owned();
        tokio::task::spawn_blocking(move || {
            let (directory, name) = Self::location(&data_dir, &scope, kind)?;
            let file = directory.lock_file(&name)?;
            file.lock().context("await project service lock release")?;
            let lock = Self {
                directory,
                name,
                file: Some(file),
                kind,
            };
            lock.release()
        })
        .await
        .context("project service lock waiter failed")?
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
                history_scope: store.restored_history_scope().map(str::to_owned),
                store_instance: store.service_instance().to_owned(),
                service_generation: generation,
                connection_secret: secret,
                schema_version: store.schema_version().await?,
            },
            address,
        })
    }

    pub(crate) fn directory(data_dir: &Path, scope: &str) -> Result<PathBuf> {
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
        Self::read_then(data_dir, scope, || {})
    }

    /// [`Self::read`] with a hook where a concurrent retirement can land
    /// between reading the held record and verifying its name.
    fn read_then(data_dir: &Path, scope: &str, between: impl FnOnce()) -> Result<Option<Self>> {
        let path = Self::path(data_dir, scope)?;
        let bytes = match crate::files::read_bytes_then(&path, HANDSHAKE_LIMIT as u64, between) {
            Ok(bytes) => bytes,
            // An absent name is not yet published. So is a held record that a
            // successor's publication over the name unlinked before its verify,
            // which reads as unlinked on Windows as on Unix.
            Err(error) if is_not_found(&error) => return Ok(None),
            // A private record that replaced the held one, still linked, before
            // its verify is a successor owner publishing during handover, which
            // readiness treats as keep waiting: the next readiness read, or for
            // maintenance the next request, finds the new record.
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(kuru_platform::fs::is_name_replaced) =>
            {
                return Ok(None);
            }
            // Retirement renames the record away before its object can be
            // marked for deletion, so a read that failed only because it met
            // a retiring owner (on Windows, a held handle that became
            // delete-pending) finds the name absent here. Any other state,
            // including a present name, keeps the original error.
            Err(_)
                if matches!(
                    crate::files::read(&path, Privacy::OwnerOnly),
                    Err(probe) if is_not_found(&probe)
                ) =>
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
        // Move the record off its discovery name before deleting it. On
        // Windows a deleted name stays occupied while any reader still holds
        // it, and later opens of it are denied; a client reading discovery
        // at this moment must instead find no record. The fixed stage name
        // replaces any stage an interrupted retirement left behind.
        directory.rename_file(
            &directory,
            name,
            &file,
            OsStr::new(RETIRED_ENDPOINT),
            Publication::ReplaceRegular,
        )?;
        match directory.read(name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!("service endpoint name was occupied during retirement"),
        }
        match directory.remove_file(OsStr::new(RETIRED_ENDPOINT), file) {
            Ok(()) => {}
            // The deletion was requested but a reader still holds the staged
            // record, so Windows removes it when that reader closes. It is
            // off the discovery name and its generation no longer listens.
            Err(error) if error.phase == PublicationPhase::Uncertain => {
                tracing::warn!(
                    error = %error,
                    "retired memory service endpoint removal awaits a concurrent reader"
                );
            }
            Err(error) => return Err(error.into()),
        }
        owner.verify()?;
        Ok(())
    }
}

/// Private stage for a record leaving discovery; never read as an endpoint.
const RETIRED_ENDPOINT: &str = "endpoint.retired";

pub(crate) fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
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
    /// Checked persisted namespace; never attachment or driver authority.
    pub history_scope: Option<String>,
    pub store_instance: String,
    pub service_generation: String,
    pub connection_secret: String,
    pub schema_version: i32,
    /// Presented only by the client that spawned this owner, on its first
    /// attachment. Additive: omitted when absent, so every other hello
    /// serializes as before. Not a secret, but left out of `Debug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_token: Option<String>,
    /// Authenticated connection identity shared by one driver's attachments.
    /// It is not permission without the owner's exact selected claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<uuid::Uuid>,
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
    /// Checked persisted namespace; never attachment or driver authority.
    pub history_scope: Option<String>,
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
            history_scope: self.history_scope.clone(),
            store_instance: self.store_instance.clone(),
            service_generation: self.service_generation.clone(),
            connection_secret: self.connection_secret.clone(),
            schema_version: self.schema_version,
            starter_token: None,
            client_id: None,
        }
    }

    pub fn verify(&self, hello: &ClientHello) -> std::result::Result<(), HandshakeRejection> {
        if hello.version.major != self.version.major
            || hello.version.minor > self.version.minor
            || hello.version.minor < PROTOCOL_MINOR
        {
            return Err(HandshakeRejection::Protocol);
        }
        if hello.project_path != self.project_path
            || hello.project_scope != self.project_scope
            || hello.history_scope != self.history_scope
        {
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
    Ok(accept_handshake_presenting(stream, authority)
        .await?
        .map(|_| ()))
}

/// [`accept_handshake`] that also returns the starter token an admitted
/// client presented, if any.
pub(crate) async fn accept_handshake_presenting<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
) -> Result<std::result::Result<Option<String>, HandshakeRejection>> {
    Ok(accept_handshake_identified(stream, authority)
        .await?
        .map(|hello| hello.starter_token))
}

pub(crate) async fn accept_handshake_identified<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
) -> Result<std::result::Result<ClientHello, HandshakeRejection>> {
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
    Ok(decision.map(|()| hello))
}

/// Verify that the peer accepted exactly the generation requested. The
/// connection is discarded by the caller on any failure.
pub async fn connect_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
) -> Result<()> {
    connect_handshake_presenting(stream, authority, None).await
}

/// [`connect_handshake`] for the starter's attachment to the owner it just
/// spawned, which presents that owner's starter token.
async fn connect_handshake_presenting<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    starter_token: Option<uuid::Uuid>,
) -> Result<()> {
    connect_handshake_identified(stream, authority, starter_token, None).await
}

async fn connect_handshake_identified<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    starter_token: Option<uuid::Uuid>,
    client_id: Option<uuid::Uuid>,
) -> Result<()> {
    ensure!(
        authority.version.major == PROTOCOL_MAJOR && authority.version.minor >= PROTOCOL_MINOR,
        "memory service protocol is incompatible; close the active Kuru session or use its matching version"
    );
    let mut hello = authority.hello();
    hello.starter_token = starter_token.map(|token| token.to_string());
    hello.client_id = client_id;
    write_frame(stream, &hello, HANDSHAKE_LIMIT, HANDSHAKE_TIMEOUT).await?;
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

/// Windows `ERROR_PIPE_NOT_CONNECTED`: the server closed the pipe instance
/// this client had connected to, which std leaves uncategorized.
#[cfg(windows)]
const ERROR_PIPE_NOT_CONNECTED: i32 = 233;

/// Whether the peer closed this connection: a broken pipe, a reset, an EOF
/// or a socket no longer connected. A stream whose connect succeeded loses
/// its connection only to its peer. Darwin reports a peer disconnect that
/// lands while its write path has the socket unlocked as ENOTCONN, and one
/// before that write as EPIPE; both are this observation.
///
/// Callers classify only connects and handshakes, before any request is
/// sent, or an owner's own attachment diagnostics. A failed request
/// exchange keeps its uncertain-write handling; this is never mutation proof.
pub fn is_peer_closed(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|error| {
            #[cfg(windows)]
            if error.raw_os_error() == Some(ERROR_PIPE_NOT_CONNECTED) {
                return true;
            }
            matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof
            )
        })
}

#[cfg(test)]
mod open_timeline_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{await_owner_release, expect_events, next_event, observed};
    use std::io::{Seek, SeekFrom, Write};
    use tokio::io::duplex;

    const FIXTURE_DIAGNOSTIC_TAIL_BYTES: u64 = 4 * 1024;

    /// An owner-side step of one write request that a test awaits: its
    /// receipt registration precedes its write, its settlement pause follows
    /// it, and its serving task ends after it (or at once on an abort or a
    /// departed client). The write spends at most its write budget
    /// (`QUERY_TIMEOUT`, taken by `write_deadline`).
    const OWNER_STEP_WITHIN: Duration = crate::store::QUERY_TIMEOUT;

    /// A failed start hands its owner's stderr, collected privately for its
    /// stages, to the test's owner diagnostic: appended whole from the start,
    /// after what the diagnostic already held, and bounded.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_failed_start_forwards_its_private_startup_diagnostic() {
        let mut diagnostic = tempfile::tempfile().unwrap();
        // The owner's writes leave the shared offset at the end.
        diagnostic
            .write_all(b"memory startup stage: OpeningDatabase\nError: owner failure\n")
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner-diagnostic.log");
        std::fs::write(&path, b"earlier owner\n").unwrap();
        let mut forward = File::options().append(true).open(&path).unwrap();
        forward_startup_diagnostic(&mut diagnostic, &mut forward);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "earlier owner\nmemory startup stage: OpeningDatabase\nError: owner failure\n"
        );

        let mut large = tempfile::tempfile().unwrap();
        large.set_len(STARTUP_DIAGNOSTIC_FORWARD_LIMIT + 1).unwrap();
        large.seek(SeekFrom::End(0)).unwrap();
        let mut bounded = tempfile::tempfile().unwrap();
        forward_startup_diagnostic(&mut large, &mut bounded);
        assert_eq!(
            bounded.metadata().unwrap().len(),
            STARTUP_DIAGNOSTIC_FORWARD_LIMIT
        );
    }

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

    /// Reads only the bounded tail of a child's file-backed stderr. A file
    /// never applies back-pressure to the child, so nothing needs draining
    /// while the child runs.
    fn fixture_diagnostic_tail(file: &mut File, label: &str) -> Result<String> {
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(
            length.saturating_sub(FIXTURE_DIAGNOSTIC_TAIL_BYTES),
        ))?;
        let mut bytes = Vec::new();
        file.take(FIXTURE_DIAGNOSTIC_TAIL_BYTES)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            Ok(format!("{label} stderr remained empty"))
        } else {
            Ok(format!(
                "{label} stderr tail: {}",
                String::from_utf8_lossy(&bytes)
            ))
        }
    }

    /// Waits for a starter fixture's ready marker with the fixture's existing
    /// 20 ms poll and caller deadline. Either failure carries the bounded
    /// tail of the starter's private stderr file.
    async fn await_starter_ready(
        mut try_wait: impl FnMut() -> io::Result<Option<std::process::ExitStatus>>,
        ready: &Path,
        deadline: tokio::time::Instant,
        stderr: &mut File,
    ) -> Result<()> {
        while !ready.exists() {
            if let Some(status) = try_wait()? {
                let tail = fixture_diagnostic_tail(stderr, STARTER_STDERR_LABEL)?;
                bail!("contained memory starter exited before readiness: {status}; {tail}");
            }
            if tokio::time::Instant::now() >= deadline {
                let tail = fixture_diagnostic_tail(stderr, STARTER_STDERR_LABEL)?;
                bail!("contained memory starter did not publish readiness; {tail}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    const STARTER_STDERR_LABEL: &str = "contained memory starter";

    /// Opens a starter's private stderr file twice: an append-only handle
    /// for the child, so each of its writes lands at the end, and a separate
    /// read handle whose offset only the parent's bounded tail read moves.
    /// Neither description shares an offset with the other.
    fn open_starter_stderr(root: &Path, name: &str) -> Result<(File, File)> {
        let path = root.join(name);
        let child = File::options().create_new(true).append(true).open(&path)?;
        let parent = File::open(&path)?;
        Ok((child, parent))
    }

    fn fixture_readiness_error(
        options: &crate::store::OpenOptions,
        diagnostic: &mut File,
        stage: &'static str,
        outcome: String,
        observations: &FixtureAttachObservations,
    ) -> Result<anyhow::Error> {
        let stderr = fixture_diagnostic_tail(diagnostic, "fixture service")?;
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
    /// observation inside the caller's named outer deadline; product election
    /// treats only a busy pipe through the deadline as a transport miss,
    /// other connect timeouts remain errors, and it never silently retries
    /// an ambiguous request.
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
        let tail = fixture_diagnostic_tail(&mut file, "fixture service")?;
        assert!(tail.starts_with("fixture service stderr tail: "));
        assert!(tail.ends_with("terminal fixture cause"));
        assert_eq!(
            tail.len(),
            "fixture service stderr tail: ".len() + FIXTURE_DIAGNOSTIC_TAIL_BYTES as usize
        );
        Ok(())
    }

    /// The starter wait helper is shared with the Windows-only contained
    /// starter fixtures; a Unix child exercises the same stderr surfacing.
    #[cfg(unix)]
    #[tokio::test]
    async fn starter_wait_surfaces_the_exited_child_stderr() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let (child_stderr, mut stderr) = open_starter_stderr(root.path(), "exited-starter")?;
        let mut child = {
            let _gate = crate::spawn_gate::spawning().await;
            std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "printf 'starter cause: election failed\\n' >&2; exit 1",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(child_stderr)
                .spawn()?
        };
        let error = await_starter_ready(
            || child.try_wait(),
            &root.path().join("never-ready"),
            tokio::time::Instant::now() + Duration::from_secs(10),
            &mut stderr,
        )
        .await
        .err()
        .context("an exited starter was reported ready")?;
        let rendered = format!("{error:#}");
        ensure!(
            rendered.starts_with("contained memory starter exited before readiness: "),
            "starter exit lost its leading text: {rendered}"
        );
        ensure!(
            rendered
                .contains("contained memory starter stderr tail: starter cause: election failed"),
            "starter exit discarded the child's stderr: {rendered}"
        );

        let (stalled_child_stderr, mut stalled_stderr) =
            open_starter_stderr(root.path(), "stalled-starter")?;
        let written = root.path().join("first-write");
        let mut running = {
            let _gate = crate::spawn_gate::spawning().await;
            std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "printf 'still starting\\n' >&2; : > \"$1\"; exec /bin/sleep 10",
                    "stalled-starter",
                ])
                .arg(&written)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(stalled_child_stderr)
                .spawn()?
        };
        // The child creates this marker only after its stderr write returned,
        // so the tail read below cannot precede that write. The helper's own
        // poll and the half's existing 2 s bound synchronise on it.
        let first_write = await_starter_ready(
            || running.try_wait(),
            &written,
            tokio::time::Instant::now() + Duration::from_secs(2),
            &mut stalled_stderr,
        )
        .await
        .context("a stalled starter did not complete its first stderr write");
        // The ready marker never appears; an elapsed deadline reports at once.
        let stalled = match first_write {
            Ok(()) => await_starter_ready(
                || running.try_wait(),
                &root.path().join("never-ready"),
                tokio::time::Instant::now(),
                &mut stalled_stderr,
            )
            .await
            .err()
            .context("a stalled starter was reported ready"),
            Err(error) => Err(error),
        };
        running.kill()?;
        running.wait()?;
        let rendered = format!("{:#}", stalled?);
        ensure!(
            rendered.starts_with("contained memory starter did not publish readiness; "),
            "starter deadline lost its leading text: {rendered}"
        );
        ensure!(
            rendered.contains("contained memory starter stderr tail: still starting"),
            "starter deadline discarded the child's stderr: {rendered}"
        );
        Ok(())
    }

    #[test]
    fn readiness_split_text_is_stable() {
        let started = tokio::time::Instant::now();
        let at = |micros: u64| started + Duration::from_micros(micros);
        let split = ReadinessSplit {
            started,
            elected: at(12_700),
            probed: at(15_900),
            spawned: at(55_200),
            expired: at(1_000_400),
            polls: 10,
            last_attach: AttachMiss::TransportUnavailable,
        };
        // Differenced offsets keep the rounded phases summing to 1000 ms.
        assert_eq!(
            split.to_string(),
            "client phases: election=12ms; owner-probe=3ms; spawn=40ms; readiness=945ms; polls=10; child=running; last-attach=transport-unavailable"
        );
        assert_eq!(AttachMiss::NoEndpoint.as_str(), "no-endpoint");
        assert_eq!(AttachMiss::PeerClosed.as_str(), "peer-closed");
    }

    /// The owner timeline clause a gated starter adds at its readiness
    /// deadline, from its own owner's stream file only. Its text is what
    /// the developer documentation reads the owner phase from.
    #[test]
    fn owner_timeline_clause_text_is_stable() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        const MS: u64 = 1_000_000;
        const MAIN_UNIX: u64 = 1_700_000_000_000_000_000;
        const CREATE_START: u64 = 9_930_123_456;
        let at = |ns: u64| UNIX_EPOCH + Duration::from_nanos(ns);
        let root = crate::test_support::tempdir()?;
        let data = root.path().join("private");
        let hash = "0123456789abcdef".repeat(4);
        let scope = format!("project/{hash}");
        let directory = EndpointRecord::directory(&data, &scope)?;
        crate::files::ensure_private_directory(&directory)?;
        let token = uuid::Uuid::new_v4();
        let tag = activity::activity_tag(&token);
        let stream = directory.join(format!("open-stream-{tag}"));
        // Another owner's stream, which this starter must never read.
        crate::files::write(
            &directory.join(format!(
                "open-stream-{}",
                activity::activity_tag(&uuid::Uuid::new_v4())
            )),
            format!(
                "owner-main 0 {MAIN_UNIX}\nsupervisor-spawned {MS} {}\n",
                MAIN_UNIX + MS
            )
            .as_bytes(),
        )?;
        let lines = format!(
            "owner-main 0 {MAIN_UNIX}\n\
             owner-lock 3400000 {}\n\
             startup-lock 4900000 {}\n\
             create-start {CREATE_START} {}\n",
            MAIN_UNIX + 3_400_000,
            MAIN_UNIX + 4_900_000,
            MAIN_UNIX + CREATE_START,
        );
        // Spawned 812.5 ms before the owner's first stamp; read 20070.9 ms
        // after its last.
        let spawned = at(MAIN_UNIX - 812_500_000);
        let now = at(MAIN_UNIX + CREATE_START + 20_070 * MS + 900_000);
        let gated = |spawned: SystemTime| {
            open_timeline::with_gate_sync(true, || {
                owner_timeline_clause(&data, &scope, &tag, spawned, now)
            })
        };
        let expected = "owner timeline: owner-exec=812ms; owner-main=0 owner-lock=3 startup-lock=4 create-start=9930 (ms); last=create-start +9930ms; since-last=20070ms";

        crate::files::write(&stream, lines.as_bytes())?;
        let clause = gated(spawned);
        assert_eq!(clause.as_deref(), Some(expected));
        let clause = clause.unwrap_or_default();
        for private in [
            tag.as_str(),
            hash.as_str(),
            &token.to_string(),
            &data.display().to_string(),
        ] {
            assert!(
                !clause.contains(private),
                "the clause carries a private value: {clause}"
            );
        }

        // A trailing partial line is still being written: ignored, uncounted.
        crate::files::write(&stream, format!("{lines}supervisor-spawned 99").as_bytes())?;
        assert_eq!(gated(spawned).as_deref(), Some(expected));

        // Malformed complete lines are counted, never named.
        crate::files::write(
            &stream,
            format!(
                "Owner-Lock 1 2\nowner-lock x 3\n{lines}migrate-start 1\n\
                 a-name-longer-than-thirty-two-bytes 1 2\n"
            )
            .as_bytes(),
        )?;
        assert_eq!(
            gated(spawned).as_deref(),
            Some(format!("{expected}; skipped=4").as_str())
        );

        // Created, with no complete line yet.
        crate::files::write(&stream, b"owner-ma")?;
        assert_eq!(gated(spawned).as_deref(), Some("owner timeline: empty"));

        // A predecessor's file with this tag: its first stamp precedes this
        // starter's spawn. The offset is a whole millisecond because
        // `SystemTime` keeps 100 ns on Windows: a 1 ns offset truncates to
        // the anchor itself, which is not stale.
        crate::files::write(&stream, lines.as_bytes())?;
        assert_eq!(
            gated(at(MAIN_UNIX + MS)).as_deref(),
            Some("owner timeline: stale")
        );

        // Over the 8 KiB read limit.
        crate::files::write(&stream, "x".repeat(8 * 1024 + 1).as_bytes())?;
        assert_eq!(
            gated(spawned).as_deref(),
            Some("owner timeline: unreadable")
        );

        // An ungated starter reads nothing and adds no clause, whatever the
        // runner's own environment holds.
        crate::files::write(&stream, lines.as_bytes())?;
        assert_eq!(
            open_timeline::with_gate_sync(false, || {
                owner_timeline_clause(&data, &scope, &tag, spawned, now)
            }),
            None
        );

        std::fs::remove_file(&stream)?;
        assert_eq!(gated(spawned).as_deref(), Some("owner timeline: absent"));
        Ok(())
    }

    /// The Windows owner's explicit environment carries the timeline gate
    /// only when the starter's own gate is set, which only exactly `1` is;
    /// a test's layer overrides it in place, and no key is ever duplicated.
    #[cfg(windows)]
    #[test]
    fn the_owner_environment_forwards_only_an_exact_gate() -> Result<()> {
        use kuru_platform::windows::process::environment_key_eq;
        let system = PathBuf::from(r"C:\Windows\System32");
        let gate = |environment: &[(OsString, OsString)]| {
            environment
                .iter()
                .filter(|(key, _)| environment_key_eq(key, OsStr::new(open_timeline::ENV)))
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>()
        };
        let unique = |environment: &[(OsString, OsString)]| {
            environment.iter().enumerate().all(|(index, (key, _))| {
                environment[index + 1..]
                    .iter()
                    .all(|(other, _)| !environment_key_eq(key, other))
            })
        };
        for (gated, expected) in [(true, vec![OsString::from("1")]), (false, Vec::new())] {
            let environment = owner_environment(
                system.clone(),
                Some(OsString::from("profile-%p.profraw")),
                open_timeline::with_gate_sync(gated, open_timeline::gate_set),
                Vec::new(),
            )?;
            assert_eq!(gate(&environment), expected, "gated={gated}");
            assert!(unique(&environment));
            for key in ["SystemRoot", "WINDIR", "PATH", "LLVM_PROFILE_FILE"] {
                assert!(
                    environment
                        .iter()
                        .any(|(name, _)| environment_key_eq(name, OsStr::new(key))),
                    "{key} missing"
                );
            }
        }
        // A test's own `0`, in any case, replaces a forwarded `1`; its `1`
        // never duplicates it; the same hook from two layers stays single.
        let hook = (
            OsString::from("KURU_TEST_MEMORY_OPEN_HOLD_DIR"),
            OsString::from(r"C:\holds"),
        );
        for (layer, expected) in [
            (
                vec![(OsString::from(open_timeline::ENV), OsString::from("0"))],
                "0",
            ),
            (
                vec![(OsString::from("kuru_open_timeline"), OsString::from("0"))],
                "0",
            ),
            (
                vec![(OsString::from(open_timeline::ENV), OsString::from("1"))],
                "1",
            ),
        ] {
            let environment = owner_environment(
                system.clone(),
                None,
                true,
                vec![vec![hook.clone()], layer, vec![hook.clone()]],
            )?;
            assert_eq!(gate(&environment), [OsString::from(expected)]);
            assert!(unique(&environment));
        }
        // Duplicates within one layer are still refused, as the spawn would.
        let duplicated = vec![vec![hook.clone(), hook]];
        assert!(owner_environment(system, None, false, duplicated).is_err());
        Ok(())
    }

    /// The failure of a launched owner that stays alive without publishing
    /// an endpoint until the lowered startup bound expires in the readiness
    /// loop, with its parsed client phase split.
    #[cfg(unix)]
    struct StalledOwnerFailure {
        rendered: String,
        fields: std::collections::BTreeMap<String, String>,
        observed_ms: u128,
        held_ms: Option<u128>,
    }

    #[cfg(unix)]
    impl StalledOwnerFailure {
        fn millis(&self, name: &str) -> Result<u128> {
            let rendered = &self.rendered;
            self.fields
                .get(name)
                .and_then(|value| value.strip_suffix("ms"))
                .with_context(|| format!("split lacks {name}: {rendered}"))?
                .parse()
                .with_context(|| format!("split {name} is not a millisecond count: {rendered}"))
        }

        /// Checks shared by every stalled-owner variant: the leading text,
        /// the whole-wait sum, a plausible poll count and the last
        /// observation. Returns the four phases in order.
        fn common_phases(&self) -> Result<[u128; 4]> {
            let Self {
                rendered,
                fields,
                observed_ms,
                ..
            } = self;
            ensure!(
                rendered.starts_with("memory service readiness deadline exceeded"),
                "readiness failure lost its leading text: {rendered}"
            );
            let phases = [
                self.millis("election")?,
                self.millis("owner-probe")?,
                self.millis("spawn")?,
                self.millis("readiness")?,
            ];
            let readiness = phases[3];
            let polls: u128 = fields
                .get("polls")
                .with_context(|| format!("split lacks polls: {rendered}"))?
                .parse()?;
            let total: u128 = phases.iter().sum();
            ensure!(
                (1000..=*observed_ms).contains(&total),
                "phases sum to {total} ms outside the 1 s bound and the {observed_ms} ms observed: {rendered}"
            );
            // Each poll after the first follows one readiness sleep, so a 1 s
            // wait now reports up to about a hundred polls, not about ten. The
            // phases are differenced whole milliseconds, hence the 1 ms slack.
            let interval = READINESS_POLL_INTERVAL.as_millis();
            ensure!(
                polls >= 2 && (polls - 1) * interval <= readiness + 1,
                "poll count {polls} is implausible for {readiness} ms of readiness: {rendered}"
            );
            ensure!(
                fields.get("child").map(String::as_str) == Some("running")
                    && fields.get("last-attach").map(String::as_str) == Some("no-endpoint"),
                "readiness failure lost its last observation: {rendered}"
            );
            Ok(phases)
        }
    }

    /// Runs `attach_or_start` against a stalled owner. With `hold`, the
    /// fixture first takes that real service lock and releases it the given
    /// duration after the attempt began, measuring the release it made.
    #[cfg(unix)]
    async fn stalled_owner_failure(
        hold: Option<(ServiceLockKind, Duration)>,
    ) -> Result<StalledOwnerFailure> {
        use std::os::unix::fs::PermissionsExt as _;
        // A held real flock is released while this attempt runs, so every
        // sibling spawn is excluded for the whole fixture; that exclusive
        // guard also covers the attempt's own owner spawn. See
        // `crate::spawn_gate`. Without a hold only the spawn is guarded.
        let _exclusive = match hold {
            Some(_) => Some(crate::spawn_gate::locking_async().await),
            None => None,
        };
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
        options.config.startup_timeout_secs = 1;
        let fifo = root.path().join("release");
        nix::unistd::mkfifo(
            &fifo,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )?;
        let release = nix::fcntl::open(
            &fifo,
            nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
            nix::sys::stat::Mode::empty(),
        )?;
        let script = root.path().join("stalled-owner");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nIFS= read -r token < '{}'\n", fifo.display()),
        )?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        let held = hold
            .map(|(kind, duration)| -> Result<_> {
                let lock =
                    ServiceLock::try_acquire(&options.data_dir, &options.project_scope, kind)?
                        .context("fixture did not acquire the held service lock")?;
                Ok((lock, duration))
            })
            .transpose()?;

        let started = tokio::time::Instant::now();
        let (outcome, released) = {
            // Held across the owner spawn; see `crate::spawn_gate`.
            let _gate = match hold {
                Some(_) => None,
                None => Some(crate::spawn_gate::spawning().await),
            };
            tokio::join!(
                tokio::time::timeout(
                    Duration::from_secs(5),
                    attach_or_start(&options, &project, &script),
                ),
                async {
                    let (lock, duration) = held?;
                    tokio::time::sleep_until(started + duration).await;
                    drop(lock);
                    Some(tokio::time::Instant::now())
                },
            )
        };
        let observed_ms = started.elapsed().as_millis();
        nix::unistd::write(&release, b"finish\n")?;
        let error = outcome
            .context("stalled owner fixture exceeded its outer deadline")?
            .err()
            .context("a stalled owner was reported ready")?;
        let rendered = format!("{error:#}");
        let split = rendered
            .split_once("client phases: ")
            .map(|(_, split)| split)
            .with_context(|| {
                format!("readiness failure lacks the client phase split: {rendered}")
            })?;
        let fields = split
            .split("; ")
            .filter_map(|field| field.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        Ok(StalledOwnerFailure {
            rendered,
            fields,
            observed_ms,
            held_ms: released.map(|instant| instant.duration_since(started).as_millis()),
        })
    }

    /// A launched owner that stays alive without publishing an endpoint
    /// exhausts the lowered startup bound in the readiness loop.
    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_deadline_reports_the_client_phase_split() -> Result<()> {
        let failure = stalled_owner_failure(None).await?;
        let [election, probe, spawn, readiness] = failure.common_phases()?;
        ensure!(
            readiness > election + probe + spawn,
            "a stalled owner's wait was not attributed to the readiness loop: {}",
            failure.rendered
        );
        Ok(())
    }

    /// Time spent waiting for a busy owner lock after election is reported
    /// as owner-probe, not election. The hold runs on the real clock, so the
    /// stalled owner reaches its fifo read within the remaining budget and
    /// the fixture's release token reaches it, as in the unheld variant.
    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_split_attributes_a_held_owner_lock_to_owner_probe() -> Result<()> {
        let failure =
            stalled_owner_failure(Some((ServiceLockKind::Owner, Duration::from_millis(300))))
                .await?;
        let [election, probe, _, _] = failure.common_phases()?;
        let held = failure
            .held_ms
            .context("owner lock hold was not measured")?;
        ensure!(
            probe * 4 >= held * 3 && election < held,
            "a {held} ms owner lock hold was not attributed to owner-probe: {}",
            failure.rendered
        );
        Ok(())
    }

    /// Time spent waiting for a busy start lock is reported as election.
    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_split_attributes_a_held_start_lock_to_election() -> Result<()> {
        let failure =
            stalled_owner_failure(Some((ServiceLockKind::Start, Duration::from_millis(300))))
                .await?;
        let [election, probe, _, _] = failure.common_phases()?;
        let held = failure
            .held_ms
            .context("start lock hold was not measured")?;
        ensure!(
            election * 4 >= held * 3 && probe < held,
            "a {held} ms start lock hold was not attributed to election: {}",
            failure.rendered
        );
        Ok(())
    }

    /// The readiness loop sleeps exactly `READINESS_POLL_INTERVAL` between
    /// polls and attaches on the poll after an endpoint appears. The spawned
    /// owner never publishes; the test publishes a real listener and endpoint
    /// record from the per-poll hook after failed poll `PUBLISH_AFTER`.
    ///
    /// Time is paused, and a miss before publication awaits nothing but its
    /// sleep, so the recorded gaps are the cadence exactly. The clock resumes
    /// before publication: the real socket handshake then runs on real time,
    /// where an idle wait for the peer cannot auto-advance the paused clock
    /// into the handshake deadline.
    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn readiness_polls_every_interval_and_attaches_on_the_next_poll() -> Result<()> {
        use std::cell::RefCell;
        use std::os::unix::fs::PermissionsExt as _;
        use std::rc::Rc;
        const PUBLISH_AFTER: u32 = 5;
        type Served = tokio::task::JoinHandle<Result<Option<String>>>;
        type Published = (ServiceLock, EndpointRecord, Served);
        // The hook takes a real owner flock while this test's spawned owner
        // exists, so every sibling spawn is excluded for the whole fixture;
        // that exclusive guard also covers the client's own owner spawn. See
        // `crate::spawn_gate`.
        let _exclusive = crate::spawn_gate::locking_async().await;
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
        let options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        let fifo = root.path().join("release");
        nix::unistd::mkfifo(
            &fifo,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )?;
        let release = nix::fcntl::open(
            &fifo,
            nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
            nix::sys::stat::Mode::empty(),
        )?;
        let script = root.path().join("silent-owner");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nIFS= read -r token < '{}'\n", fifo.display()),
        )?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        let mut endpoint_authority = authority();
        endpoint_authority.project_path = project_path_bytes(&project);
        endpoint_authority.project_scope = scope.clone();

        let missed: Rc<RefCell<Vec<(u32, tokio::time::Instant)>>> = Rc::default();
        let published: Rc<RefCell<Option<Published>>> = Rc::default();
        let hook = {
            let (missed, published) = (Rc::clone(&missed), Rc::clone(&published));
            let (data, scope) = (data.clone(), scope.clone());
            let authority = endpoint_authority.clone();
            move |polls: u32| {
                missed
                    .borrow_mut()
                    .push((polls, tokio::time::Instant::now()));
                if polls != PUBLISH_AFTER {
                    return;
                }
                tokio::time::resume();
                let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)
                    .expect("owner lock probe")
                    .expect("the spawned fixture owner holds no owner lock");
                let (mut listener, address) =
                    ServiceListener::bind(&data, &scope).expect("bind fixture listener");
                let endpoint = EndpointRecord {
                    authority: authority.clone(),
                    address,
                };
                endpoint
                    .publish(&data, &owner)
                    .expect("publish fixture endpoint");
                let authority = authority.clone();
                let served = tokio::spawn(async move {
                    let mut stream = listener.accept(HANDSHAKE_TIMEOUT).await?;
                    accept_handshake_presenting(&mut stream, &authority)
                        .await?
                        .map_err(|reason| anyhow::anyhow!(reason.diagnostic()))
                });
                *published.borrow_mut() = Some((owner, endpoint, served));
            }
        };
        let attached = tokio::time::timeout(
            Duration::from_secs(60),
            readiness_poll_hook::MISSED.scope(
                RefCell::new(Box::new(hook)),
                attach_or_start(&options, &project, &script),
            ),
        )
        .await;
        nix::unistd::write(&release, b"finish\n")?;
        let attached = attached
            .context("cadence fixture exceeded its outer deadline")?
            .context("the client did not attach to the published endpoint")?;
        let (owner, endpoint, served) = published
            .borrow_mut()
            .take()
            .context("the hook never published an endpoint")?;
        let presented = served.await??;

        let missed = missed.borrow();
        let polls: Vec<u32> = missed.iter().map(|(polls, _)| *polls).collect();
        ensure!(
            polls == (1..=PUBLISH_AFTER).collect::<Vec<_>>(),
            "an endpoint published after poll {PUBLISH_AFTER} was not attached on the next poll: missed {polls:?}"
        );
        let gaps: Vec<Duration> = missed
            .windows(2)
            .map(|pair| pair[1].1.duration_since(pair[0].1))
            .collect();
        ensure!(
            gaps.iter().all(|gap| *gap == READINESS_POLL_INTERVAL),
            "readiness polls were not {READINESS_POLL_INTERVAL:?} apart: {gaps:?}"
        );
        ensure!(
            attached.generation() == endpoint_authority.service_generation,
            "the client attached to another generation"
        );
        ensure!(
            presented.is_some(),
            "the starting client did not present its starter token"
        );
        drop(attached);
        endpoint.retire(&data, &owner)?;
        owner.release()?;
        Ok(())
    }

    /// R1-R4: the elected starter's readiness wait against a real child
    /// owner, the test-support stand-in, on every platform. The stand-in
    /// takes no lock and publishes nothing; each test publishes the owner's
    /// tagged record itself from the per-poll hook, under the starter token
    /// it gave the starter, and no test body sleeps.
    #[cfg(feature = "test-support")]
    mod progress_wait {
        use super::*;
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;

        /// Real-time bound for a spawned stand-in to consume its release.
        const CONSUME_BOUND: Duration = Duration::from_secs(30);

        type Missed = Rc<RefCell<Vec<(u32, tokio::time::Instant)>>>;

        struct StandInOwner {
            root: crate::test_support::TempDir,
            project: PathBuf,
            data: PathBuf,
            scope: String,
            options: crate::store::OpenOptions,
            executable: PathBuf,
            release: PathBuf,
            tag: String,
            /// Whether a release has been written; the first one decides.
            released: Cell<bool>,
        }

        impl StandInOwner {
            fn new(startup_timeout_secs: u64) -> Result<Self> {
                let root = crate::test_support::tempdir()?;
                let (project, scope, data, mut options) = owner_fixture(root.path())?;
                options.config.startup_timeout_secs = startup_timeout_secs;
                let token = uuid::Uuid::new_v4();
                options.starter_token = Some(token);
                Ok(Self {
                    release: root.path().join("release"),
                    executable: crate::store::test_supervisor()?,
                    tag: activity::activity_tag(&token),
                    released: Cell::new(false),
                    root,
                    project,
                    data,
                    scope,
                    options,
                })
            }

            /// Release the stand-in with `status`, staged and then renamed
            /// onto the release name so it never reads a partial file. Only
            /// the first release is written: a stand-in that consumed it has
            /// exited, and a second file would never be consumed.
            fn release(&self, status: i32) -> Result<()> {
                if self.released.replace(true) {
                    return Ok(());
                }
                let staged = self.root.path().join("release.staged");
                std::fs::write(&staged, status.to_string())?;
                std::fs::rename(&staged, &self.release)?;
                Ok(())
            }

            fn window(&self) -> Duration {
                Duration::from_secs(self.options.config.startup_timeout_secs)
            }

            /// Wait in real time, which the paused test clock does not
            /// advance, until the stand-in has removed its release file. It
            /// removes the file only to exit at once, so a test whose body
            /// finishes within one of its polls, as these virtual-time tests
            /// do, never removes the fixture directory first and leaves the
            /// stand-in polling a vanished release until its 120 s bound.
            fn consumed(&self) -> Result<()> {
                let deadline = std::time::Instant::now() + CONSUME_BOUND;
                loop {
                    match std::fs::symlink_metadata(&self.release) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                        Err(error) => {
                            return Err(error).context("inspect the stand-in owner's release");
                        }
                        Ok(_) => {}
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "the stand-in owner did not consume its release within {CONSUME_BOUND:?}"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }

            /// Run the starter with the stand-in as its owner and `hook`
            /// after each failed poll, recording every failed poll's instant.
            /// The stand-in is released with status 0 whatever the outcome,
            /// before any result is examined, so no failing test leaves it;
            /// once it was spawned, its release is consumed before this
            /// returns, so it never outlives the test.
            async fn start(
                &self,
                missed: &Missed,
                mut hook: impl FnMut(u32) + 'static,
                bound: Duration,
            ) -> Result<Result<ServiceAttachment>> {
                let recorded = Rc::clone(missed);
                let hook = move |polls: u32| {
                    recorded
                        .borrow_mut()
                        .push((polls, tokio::time::Instant::now()));
                    hook(polls);
                };
                let environment = vec![(
                    OsString::from(SERVICE_STAND_IN_ENV),
                    self.release.clone().into_os_string(),
                )];
                let outcome = tokio::time::timeout(
                    bound,
                    activity::with_owner_environment(
                        environment,
                        readiness_poll_hook::MISSED.scope(
                            RefCell::new(Box::new(hook)),
                            attach_or_start(&self.options, &self.project, &self.executable),
                        ),
                    ),
                )
                .await;
                let released = self.release(0);
                // Polls and an attachment follow the spawn; a failure with
                // neither may precede it, and then nothing consumes the file.
                let spawned = !missed.borrow().is_empty() || matches!(outcome, Ok(Ok(_)));
                let outcome = outcome.context("stand-in owner fixture exceeded its outer deadline");
                released?;
                if spawned {
                    self.consumed()?;
                }
                outcome
            }
        }

        /// The failed polls in order, each exactly one poll interval after
        /// the previous, so every index below is derived, not assumed.
        fn derived_polls(missed: &Missed) -> Result<Vec<(u32, tokio::time::Instant)>> {
            let missed = missed.borrow().clone();
            let polls: Vec<u32> = missed.iter().map(|(polls, _)| *polls).collect();
            ensure!(
                polls.iter().copied().eq(1..=polls.len() as u32),
                "failed polls are not numbered consecutively from 1: {polls:?}"
            );
            let gaps: Vec<Duration> = missed
                .windows(2)
                .map(|pair| pair[1].1.duration_since(pair[0].1))
                .collect();
            ensure!(
                gaps.iter().all(|gap| *gap == READINESS_POLL_INTERVAL),
                "readiness polls were not {READINESS_POLL_INTERVAL:?} apart: {gaps:?}"
            );
            Ok(missed)
        }

        fn readiness_failure(error: &anyhow::Error) -> Result<&ReadinessFailure> {
            error
                .downcast_ref::<ReadinessFailure>()
                .with_context(|| format!("the readiness failure is not typed: {error:#}"))
        }

        /// R1: an owner whose record changes at least once in every window is
        /// waited for past the flat bound and attached when it publishes.
        /// The hook advances the record at polls 99, 198 and 297, each read
        /// at the next poll, 990 ms after the previous progress, and after
        /// poll 300 publishes a real listener and endpoint on the resumed
        /// clock, as the cadence test does.
        #[tokio::test(start_paused = true)]
        async fn an_advancing_owner_is_waited_for_beyond_one_window() -> Result<()> {
            const ADVANCES: [u32; 3] = [99, 198, 297];
            const PUBLISH_AFTER: u32 = 300;
            type Served = tokio::task::JoinHandle<Result<Option<String>>>;
            type Published = (ServiceLock, EndpointRecord, Served);
            // The hook takes a real owner lock while this test's spawned
            // owner exists; see `crate::spawn_gate`.
            let _exclusive = crate::spawn_gate::locking_async().await;
            let fixture = StandInOwner::new(1)?;
            let mut endpoint_authority = authority();
            endpoint_authority.project_path = project_path_bytes(&fixture.project);
            endpoint_authority.project_scope = fixture.scope.clone();

            let missed = Missed::default();
            let published: Rc<RefCell<Option<Published>>> = Rc::default();
            let hook = {
                let published = Rc::clone(&published);
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                let authority = endpoint_authority.clone();
                move |polls: u32| {
                    if let Some(advance) = ADVANCES.iter().position(|at| *at == polls) {
                        activity::write_progress_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::OpeningDatabase],
                            advance as u64 + 1,
                        )
                        .expect("publish the owner's progress");
                    }
                    if polls != PUBLISH_AFTER {
                        return;
                    }
                    tokio::time::resume();
                    let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)
                        .expect("owner lock probe")
                        .expect("the stand-in owner holds no owner lock");
                    let (mut listener, address) =
                        ServiceListener::bind(&data, &scope).expect("bind fixture listener");
                    let endpoint = EndpointRecord {
                        authority: authority.clone(),
                        address,
                    };
                    endpoint
                        .publish(&data, &owner)
                        .expect("publish fixture endpoint");
                    let authority = authority.clone();
                    let served = tokio::spawn(async move {
                        let mut stream = listener.accept(HANDSHAKE_TIMEOUT).await?;
                        accept_handshake_presenting(&mut stream, &authority)
                            .await?
                            .map_err(|reason| anyhow::anyhow!(reason.diagnostic()))
                    });
                    *published.borrow_mut() = Some((owner, endpoint, served));
                }
            };
            let outcome = fixture
                .start(&missed, hook, Duration::from_secs(60))
                .await?;
            let returned = tokio::time::Instant::now();
            let missed = derived_polls(&missed)?;
            let attached = match outcome {
                Ok(attached) => attached,
                Err(error) => bail!(
                    "an owner advancing at least once per window was abandoned after {} failed polls: {error:#}",
                    missed.len()
                ),
            };
            let (owner, endpoint, served) = published
                .borrow_mut()
                .take()
                .context("the hook never published an endpoint")?;
            let presented = served.await??;
            ensure!(
                missed.len() == PUBLISH_AFTER as usize,
                "the endpoint published after poll {PUBLISH_AFTER} was not attached on the next poll: {} failed polls",
                missed.len()
            );
            let first = missed.first().context("no failed poll was recorded")?.1;
            ensure!(
                returned.duration_since(first) >= fixture.window() * 3,
                "the wait ended {:?} after spawn, inside three windows",
                returned.duration_since(first)
            );
            ensure!(
                attached.generation() == endpoint_authority.service_generation
                    && presented.is_some(),
                "the starter attached to another generation or without its token"
            );
            drop(attached);
            endpoint.retire(&fixture.data, &owner)?;
            owner.release()?;
            Ok(())
        }

        /// R2: an owner whose record last changed at poll 51 (written by
        /// the hook of poll 50) is abandoned one window later, at poll 151,
        /// naming the stage it last published.
        #[tokio::test(start_paused = true)]
        async fn a_stalled_owner_is_abandoned_naming_its_stage() -> Result<()> {
            const WRITE_AFTER: u32 = 50;
            let fixture = StandInOwner::new(1)?;
            let missed = Missed::default();
            let hook = {
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                move |polls: u32| {
                    if polls == WRITE_AFTER {
                        activity::write_progress_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::ExtractingEmbeddedRuntime],
                            1,
                        )
                        .expect("publish the owner's progress");
                    }
                }
            };
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(60))
                    .await?
            };
            let missed = derived_polls(&missed)?;
            let error = outcome
                .err()
                .context("a stalled owner was reported ready")?;
            let rendered = format!("{error:#}");
            let ReadinessFailure::Stalled {
                last_stage, since, ..
            } = readiness_failure(&error)?
            else {
                bail!("a stalled owner was not reported as a stall: {rendered}");
            };
            ensure!(
                *last_stage == Some(MemoryOpenStage::ExtractingEmbeddedRuntime),
                "a stalled owner was abandoned without the stage it last published: {rendered}"
            );
            ensure!(
                missed.len() == 150 && *since == fixture.window(),
                "a stalled owner was abandoned after {} failed polls and {since:?} without progress, not one window after its progress at poll 51: {rendered}",
                missed.len()
            );
            ensure!(
                rendered.starts_with("memory service readiness deadline exceeded; "),
                "the stall lost its leading text: {rendered}"
            );
            Ok(())
        }

        /// R3: an owner that exits after its record named a stage fails the
        /// wait at the poll that observes the exit, with its status and that
        /// stage. Real clock and a 300 s window: the stand-in's exit, which
        /// the hook releases at poll 6 after poll 6 read the record written
        /// at poll 5, races only that window, so the outcome is
        /// deterministic by budget without any sleep. A stand-in that never
        /// exited would end as a stall naming the stage, not a hang.
        #[tokio::test]
        async fn an_exited_owner_fails_at_once_with_its_stage() -> Result<()> {
            const WRITE_AFTER: u32 = 5;
            const RELEASE_AFTER: u32 = 6;
            let fixture = Rc::new(StandInOwner::new(300)?);
            let missed = Missed::default();
            let hook = {
                let fixture = Rc::clone(&fixture);
                move |polls: u32| {
                    if polls == WRITE_AFTER {
                        activity::write_progress_record(
                            &fixture.data,
                            &fixture.scope,
                            &fixture.tag,
                            &[MemoryOpenStage::CreatingDatabase],
                            1,
                        )
                        .expect("publish the owner's progress");
                    }
                    if polls == RELEASE_AFTER {
                        fixture.release(3).expect("release the stand-in owner");
                    }
                }
            };
            let started = tokio::time::Instant::now();
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(330))
                    .await?
            };
            let elapsed = started.elapsed();
            let polls = missed.borrow().len();
            let error = outcome
                .err()
                .context("an exited owner was reported ready")?;
            let rendered = format!("{error:#}");
            let ReadinessFailure::OwnerExited {
                status, last_stage, ..
            } = readiness_failure(&error)?
            else {
                bail!("an exited owner was not reported as exited: {rendered}");
            };
            ensure!(
                status.code() == Some(3),
                "the exited owner's status was not reported: {rendered}"
            );
            ensure!(
                *last_stage == Some(MemoryOpenStage::CreatingDatabase),
                "an exited owner was reported without the stage read before its exit: {rendered}"
            );
            ensure!(
                polls >= RELEASE_AFTER as usize && elapsed < fixture.window(),
                "the exit was observed after {polls} failed polls and {elapsed:?}: {rendered}"
            );
            ensure!(
                rendered.starts_with("memory service exited before readiness: "),
                "the exit lost its leading text: {rendered}"
            );
            Ok(())
        }

        /// R4: a record the starter has read and that its owner then retires
        /// ends the wait at the next poll, while the owner stays alive. The
        /// hook writes the record at poll 5, read at poll 6, and retires it
        /// with the owner's own retirement at poll 7, so poll 8 fails.
        #[tokio::test(start_paused = true)]
        async fn a_retired_record_fails_at_once() -> Result<()> {
            const WRITE_AFTER: u32 = 5;
            const RETIRE_AFTER: u32 = 7;
            let fixture = StandInOwner::new(1)?;
            let missed = Missed::default();
            let retired: Rc<RefCell<Option<Result<()>>>> = Rc::default();
            let hook = {
                let retired = Rc::clone(&retired);
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                move |polls: u32| {
                    if polls == WRITE_AFTER {
                        activity::write_progress_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::CreatingDatabase],
                            1,
                        )
                        .expect("publish the owner's progress");
                    }
                    if polls == RETIRE_AFTER {
                        *retired.borrow_mut() =
                            Some(activity::retire_tagged_record(&data, &scope, &tag));
                    }
                }
            };
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(60))
                    .await?
            };
            let missed = derived_polls(&missed)?;
            let error = outcome
                .err()
                .context("an owner whose record was retired was reported ready")?;
            let rendered = format!("{error:#}");
            let ReadinessFailure::OwnerRetired { last_stage, .. } = readiness_failure(&error)?
            else {
                bail!(
                    "a retired record did not end the wait at once; it ended after {} failed polls: {rendered}",
                    missed.len()
                );
            };
            ensure!(
                *last_stage == Some(MemoryOpenStage::CreatingDatabase),
                "the retirement was reported without the stage read before it: {rendered}"
            );
            ensure!(
                missed.len() == RETIRE_AFTER as usize,
                "the retirement at poll {RETIRE_AFTER} was observed after {} failed polls: {rendered}",
                missed.len()
            );
            retired
                .borrow_mut()
                .take()
                .context("the hook never retired the record")?
                .context("the owner's retirement of its record failed")?;
            ensure!(
                rendered.starts_with("memory service ended its open before readiness; "),
                "the retirement lost its leading text: {rendered}"
            );
            Ok(())
        }

        /// A stand-in whose test finishes at once has consumed its release,
        /// and so is exiting, before its fixture's start returns. These
        /// tests run in paused virtual time, so the body can end and remove
        /// the fixture directory before the stand-in's first real 10 ms
        /// poll; an instrumented stand-in left polling a vanished release
        /// lived on for its 120 s bound and wrote its coverage profile while
        /// a coverage partition was exporting.
        #[tokio::test(start_paused = true)]
        async fn a_stand_in_has_consumed_its_release_when_its_start_returns() -> Result<()> {
            let fixture = StandInOwner::new(1)?;
            let missed = Missed::default();
            let hook = {
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                move |polls: u32| {
                    if polls == 1 {
                        activity::write_failing_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::CreatingDatabase],
                            1,
                            "refused",
                        )
                        .expect("publish the owner's failing mark");
                    }
                }
            };
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(60))
                    .await?
            };
            ensure!(
                outcome.is_err(),
                "an owner whose record is marked failing was reported ready"
            );
            ensure!(
                !missed.borrow().is_empty(),
                "the starter never polled the stand-in"
            );
            match std::fs::symlink_metadata(&fixture.release) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).context("inspect the stand-in owner's release"),
                Ok(_) => {
                    bail!("the stand-in's release was still unconsumed when its start returned")
                }
            }
        }

        /// A record its owner marks failing ends the wait at the poll that
        /// reads it, with the recorded reason, while the owner stays alive
        /// and before any window could end. The hook writes progress at
        /// poll 5, read at poll 6, and the failing mark at poll 7, so poll 8
        /// fails.
        #[tokio::test(start_paused = true)]
        async fn a_failing_record_fails_at_once_with_its_reason() -> Result<()> {
            const WRITE_AFTER: u32 = 5;
            const FAIL_AFTER: u32 = 7;
            const REASON: &str = "open active main pool: refused";
            let fixture = StandInOwner::new(1)?;
            let missed = Missed::default();
            let hook = {
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                move |polls: u32| {
                    if polls == WRITE_AFTER {
                        activity::write_progress_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::CreatingDatabase],
                            1,
                        )
                        .expect("publish the owner's progress");
                    }
                    if polls == FAIL_AFTER {
                        activity::write_failing_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::CreatingDatabase],
                            2,
                            REASON,
                        )
                        .expect("publish the owner's failing mark");
                    }
                }
            };
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(60))
                    .await?
            };
            let missed = derived_polls(&missed)?;
            let error = outcome
                .err()
                .context("an owner whose record is marked failing was reported ready")?;
            let rendered = format!("{error:#}");
            let ReadinessFailure::OwnerFailed {
                reason,
                last_stage,
                seen,
                ..
            } = readiness_failure(&error)?
            else {
                bail!(
                    "a failing record did not end the wait at once; it ended after {} failed polls: {rendered}",
                    missed.len()
                );
            };
            ensure!(
                reason == REASON
                    && *last_stage == Some(MemoryOpenStage::CreatingDatabase)
                    && *seen == 2,
                "the failing mark was reported without its reason, stage or count: {rendered}"
            );
            ensure!(
                missed.len() == FAIL_AFTER as usize,
                "the failing mark written at poll {FAIL_AFTER} was observed after {} failed polls: {rendered}",
                missed.len()
            );
            ensure!(
                rendered.starts_with(&format!(
                    "memory service open failed before readiness: {REASON}; owner progress: "
                )),
                "the failing mark lost its leading text: {rendered}"
            );
            Ok(())
        }

        /// A stale record carrying the starter's own tag, left before the
        /// spawn, counts once at the first poll and never again; the owner's
        /// first write after it has a lower count and still counts, so the
        /// wait ends one window after that write is read at poll 51.
        #[tokio::test(start_paused = true)]
        async fn a_stale_record_counts_once_and_a_lower_count_still_counts() -> Result<()> {
            const WRITE_AFTER: u32 = 50;
            let fixture = StandInOwner::new(1)?;
            activity::write_progress_record(
                &fixture.data,
                &fixture.scope,
                &fixture.tag,
                &[MemoryOpenStage::UpgradingDatabase],
                80,
            )?;
            let missed = Missed::default();
            let hook = {
                let (data, scope, tag) = (
                    fixture.data.clone(),
                    fixture.scope.clone(),
                    fixture.tag.clone(),
                );
                move |polls: u32| {
                    if polls == WRITE_AFTER {
                        activity::write_progress_record(
                            &data,
                            &scope,
                            &tag,
                            &[MemoryOpenStage::PreparingDatabase],
                            1,
                        )
                        .expect("publish the owner's progress");
                    }
                }
            };
            let outcome = {
                // Held across the owner spawn; see `crate::spawn_gate`.
                let _gate = crate::spawn_gate::spawning().await;
                fixture
                    .start(&missed, hook, Duration::from_secs(60))
                    .await?
            };
            let missed = derived_polls(&missed)?;
            let error = outcome
                .err()
                .context("a stalled owner was reported ready")?;
            let rendered = format!("{error:#}");
            let ReadinessFailure::Stalled {
                last_stage,
                seen,
                since,
                ..
            } = readiness_failure(&error)?
            else {
                bail!("a stalled owner was not reported as a stall: {rendered}");
            };
            ensure!(
                *last_stage == Some(MemoryOpenStage::PreparingDatabase) && *seen == 1,
                "the owner's lower count after a stale record was not read as progress: {rendered}"
            );
            ensure!(
                missed.len() == 150 && *since == fixture.window(),
                "the wait ended after {} failed polls and {since:?} without progress, not one window after poll 51: {rendered}",
                missed.len()
            );
            Ok(())
        }
    }

    /// One owner event hold: `<event>.entered` and `<event>.release` fifos in
    /// the hold directory, each opened here read-write and non-blocking, so
    /// the owner's non-blocking opens find a peer and neither end reports a
    /// hang-up while this test holds them.
    #[cfg(unix)]
    struct EventHold {
        event: &'static str,
        entered: std::os::fd::OwnedFd,
        release: std::os::fd::OwnedFd,
    }

    #[cfg(unix)]
    impl EventHold {
        fn create(directory: &Path, event: &'static str) -> Result<Self> {
            let fifo = |suffix: &str| -> Result<std::os::fd::OwnedFd> {
                let path = directory.join(format!("{event}.{suffix}"));
                nix::unistd::mkfifo(
                    &path,
                    nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
                )?;
                Ok(nix::fcntl::open(
                    &path,
                    nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
                    nix::sys::stat::Mode::empty(),
                )?)
            };
            Ok(Self {
                event,
                entered: fifo("entered")?,
                release: fifo("release")?,
            })
        }

        /// A blocking wait, on its own handle, for the owner's one entered
        /// byte, bounded by `bound` on the real clock.
        fn entered_waiter(&self, bound: Duration) -> Result<impl FnOnce() -> Result<()> + use<>> {
            let (event, entered) = (self.event, self.entered.try_clone()?);
            Ok(move || {
                use std::os::fd::AsFd as _;
                let deadline = std::time::Instant::now() + bound;
                loop {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let timeout = nix::poll::PollTimeout::try_from(remaining)
                        .unwrap_or(nix::poll::PollTimeout::MAX);
                    let mut fds = [nix::poll::PollFd::new(
                        entered.as_fd(),
                        nix::poll::PollFlags::POLLIN,
                    )];
                    match nix::poll::poll(&mut fds, timeout) {
                        Ok(0) => bail!(
                            "the owner never reached the {event} hold within {} s (no timeline hold hook, or the owner stopped before it)",
                            bound.as_secs()
                        ),
                        Ok(_) => break,
                        // Interrupted, not expired: wait out the remainder.
                        Err(nix::errno::Errno::EINTR) => {}
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("poll the {event} entered fifo"));
                        }
                    }
                }
                let mut byte = [0_u8; 1];
                nix::unistd::read(&entered, &mut byte)
                    .with_context(|| format!("read the {event} entered byte"))?;
                Ok(())
            })
        }

        fn release(&self) -> Result<()> {
            nix::unistd::write(&self.release, b"r")
                .with_context(|| format!("release the {} hold", self.event))?;
            Ok(())
        }
    }

    /// A gated starter whose real gated owner is held right after streaming
    /// `create-start` reaches its default readiness deadline and names that
    /// event as the owner's last.
    ///
    /// The client clock is paused only after the real-clock setup. A
    /// blocking task waiting for the owner's entered byte is spawned before
    /// the open is first polled; while it runs the paused clock cannot
    /// auto-advance, so the starter's poll sleeps cannot reach the deadline
    /// before the owner has streamed `create-start`. Its bound is the
    /// configured startup budget on the real clock: reaching `create-start`
    /// is a strict prefix of the open the product requires within that
    /// budget. Afterwards virtual time runs to the deadline while the owner
    /// stays held. `owner-exec` and `since-last` are real-clock differences
    /// while the client phases are virtual, so only their presence is
    /// asserted.
    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_deadline_names_the_owner_event_it_was_held_at() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        let root = crate::test_support::tempdir()?;
        let (project, scope, data, mut options) = owner_fixture(root.path())?;
        let executable = options
            .supervisor
            .clone()
            .context("fixture supervisor absent")?;
        let token = uuid::Uuid::new_v4();
        options.starter_token = Some(token);
        let budget = Duration::from_secs(options.config.startup_timeout_secs);
        let holds = root.path().join("holds");
        std::fs::create_dir(&holds)?;
        let held = EventHold::create(&holds, "create-start")?;
        // Released ahead, so the owner passes it at once and only signals
        // that its endpoint is published.
        let published = EventHold::create(&holds, "endpoint-published")?;
        published.release()?;
        let environment = vec![
            (OsString::from(open_timeline::ENV), OsString::from("1")),
            (
                OsString::from(open_timeline::hold::DIR_ENV),
                holds.clone().into_os_string(),
            ),
        ];
        let reached_hold = held.entered_waiter(budget)?;

        // Held across the owner spawn; see `crate::spawn_gate`.
        let spawn_gate = crate::spawn_gate::spawning().await;
        tokio::time::pause();
        let inhibitor = tokio::task::spawn_blocking(reached_hold);
        let (outcome, reached) = activity::with_owner_environment(
            environment,
            open_timeline::with_gate(true, async {
                tokio::join!(attach_or_start(&options, &project, &executable), async {
                    let reached = inhibitor
                        .await
                        .context("the hold waiter panicked")
                        .and_then(|reached| reached);
                    // Never handshake under a paused clock once the hold
                    // was missed.
                    if reached.is_err() {
                        tokio::time::resume();
                    }
                    reached
                })
            }),
        )
        .await;
        if reached.is_ok() {
            tokio::time::resume();
        }
        drop(spawn_gate);

        let verdict = (|| -> Result<()> {
            reached
                .as_ref()
                .map_err(|error| anyhow::anyhow!("{error:#}"))?;
            let error = outcome
                .as_ref()
                .err()
                .context("an owner held at create-start was reported ready")?;
            let rendered = format!("{error:#}");
            ensure!(
                rendered.starts_with("memory service readiness deadline exceeded"),
                "readiness failure lost its leading text: {rendered}"
            );
            let (_, clause) = rendered
                .split_once("owner timeline: ")
                .with_context(|| format!("the deadline lacks the owner timeline: {rendered}"))?;
            let (clause, split) = clause.split_once("; client phases: ").with_context(|| {
                format!("the owner timeline precedes no client split: {rendered}")
            })?;
            let fields: Vec<&str> = clause.split("; ").collect();
            ensure!(
                fields.iter().any(|field| field.starts_with("owner-exec=")),
                "the owner timeline lacks owner-exec: {rendered}"
            );
            ensure!(
                fields
                    .iter()
                    .any(|field| field.starts_with("last=create-start ")),
                "the owner timeline does not end at create-start: {rendered}"
            );
            let events: Vec<&str> = fields
                .iter()
                .find_map(|field| field.strip_suffix(" (ms)"))
                .with_context(|| format!("the owner timeline lists no events: {rendered}"))?
                .split(' ')
                .filter_map(|event| event.split_once('=').map(|(name, _)| name))
                .collect();
            let position = |name: &str| events.iter().position(|event| *event == name);
            let order = ["owner-main", "owner-lock", "startup-lock", "create-start"].map(position);
            ensure!(
                order.iter().all(Option::is_some) && order.is_sorted(),
                "the owner timeline lacks the held prefix in order: {rendered}"
            );
            ensure!(
                !clause.contains("supervisor-spawned"),
                "a held owner streamed an engine start: {rendered}"
            );
            let split: std::collections::BTreeMap<&str, &str> = split
                .split("; ")
                .filter_map(|field| field.split_once('='))
                .collect();
            ensure!(
                split.get("child") == Some(&"running")
                    && split.get("last-attach") == Some(&"no-endpoint")
                    && split.contains_key("readiness"),
                "the client split no longer parses: {rendered}"
            );
            Ok(())
        })();

        // Cleanup on every path, on the real clock.
        held.release()?;
        let attachment = outcome.ok();
        let deadline = crate::test_support::fixture_deadline(1, 0);
        let cleanup = tokio::time::timeout(deadline, async {
            if reached.is_ok() {
                // The owner signals this hold only after publishing its
                // endpoint; its starter attaches so it closes at once.
                let bound = crate::test_support::fresh_open_budget();
                tokio::task::spawn_blocking(published.entered_waiter(bound)?).await??;
                drop(attach_raw(&data, &scope, Some(token)).await?);
                await_owner_release(&options).await?;
            }
            drop(attachment);
            crate::test_support::await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("held-owner cleanup exceeded its {deadline:?} deadline"))
        .and_then(|retired| retired.context("retire the held owner"));
        // The verdict first: on a missed hold it carries the diagnostic.
        verdict.and(cleanup)
    }

    #[tokio::test]
    async fn inspection_waits_for_a_booting_owner_without_starting_dolt() -> Result<()> {
        // Held for the whole test: it takes and releases a real owner flock and
        // never spawns, so no sibling test's child inherits that lock's
        // description; see `crate::spawn_gate`. The fixture root's teardown
        // reads quiescence records and never probes this lock.
        let _gate = crate::spawn_gate::locking_async().await;
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
            format!("{error:#}")
                .contains("was still shutting down or did not publish a valid endpoint"),
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

    // T14, service half: a published pipe whose only instance stays busy
    // through the connect deadline is a transport miss for an electing
    // client and for maintenance, not the fatal connect timeout.
    #[cfg(windows)]
    #[tokio::test]
    async fn busy_pipe_through_the_deadline_is_a_transport_miss() -> Result<()> {
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
        let options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let (listener, address) = ServiceListener::bind(&data, &scope)?;
        let mut endpoint_authority = authority();
        endpoint_authority.project_path = project_path_bytes(&project);
        endpoint_authority.project_scope = scope.clone();
        let endpoint = EndpointRecord {
            authority: endpoint_authority,
            address: address.clone(),
        };
        endpoint.publish(&data, &owner)?;
        // Never accepted: this client holds the listener's only instance, so
        // every later connect retry sees a busy pipe until its deadline.
        let occupant = connect_local(&data, &scope, &address, HANDSHAKE_TIMEOUT).await?;

        let observed = try_attach_observed(&data, &scope, &project, None)
            .await
            .context("an electing client failed on a busy pipe")?;
        ensure!(
            matches!(observed, Err(AttachMiss::TransportUnavailable)),
            "a busy pipe was not a transport miss for an electing client"
        );
        ensure!(
            request_idle_retirement(&options)
                .await
                .context("maintenance failed on a busy pipe")?
                == RetirementReply::NoEndpoint,
            "maintenance reached an owner through a busy pipe"
        );

        drop(occupant);
        drop(listener);
        endpoint.retire(&data, &owner)?;
        Ok(())
    }

    #[tokio::test]
    async fn purge_refuses_a_live_service_owner_before_writing_intent() -> Result<()> {
        // Held for the whole test: it takes and releases a real owner flock and
        // never spawns, so no sibling test's child inherits that lock's
        // description; see `crate::spawn_gate`. The fixture root's teardown
        // reads quiescence records and never probes this lock.
        let _gate = crate::spawn_gate::locking_async().await;
        let data = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "a".repeat(64));
        let mut options = crate::OpenOptions::new(data.path().to_owned(), scope.clone());
        // Shorter than the owner's close budget, which therefore sets the
        // maintenance deadline.
        options.config.startup_timeout_secs = 1;
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let bound = maintenance_deadline(&options) + Duration::from_secs(2);
        // A bare owner lock publishes no endpoint, so every retirement request
        // is a record read with no socket; paused time elapses the deadline.
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let error = tokio::time::timeout(bound, crate::MemoryStore::purge(options))
            .await
            .context("purge did not respect its owner deadline")?
            .expect_err("purge must not run while the service owns the store");
        let waited = started.elapsed();
        tokio::time::resume();
        ensure!(
            format!("{error:#}").contains("memory service owner is still active"),
            "purge did not explain the live owner: {error:#}"
        );
        ensure!(
            waited >= crate::server::close_budget(),
            "purge gave up after {waited:?}, inside the owner's close budget"
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
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, retired by maintenance.
        let deadline = crate::test_support::FixtureDeadline::start(
            crate::test_support::fixture_deadline(1, 0),
            "idle-owner maintenance fixture",
        );
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
        let outcome = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    // Exercise immediate maintenance against the product's
                    // thirty-second retention, not the never-reached fixture policy.
                    let knobs = owner.knobs();
                    served.serve_with(owner, knobs)?;
                    async {
                        let mut client = try_attach(&data, &scope, &project)
                            .await?
                            .context("fixture owner did not accept a client")?;

                        // The purge's permit acquisition answers within the
                        // product's maintenance deadline.
                        let refused = tokio::time::timeout(
                            maintenance_deadline(&options),
                            crate::MemoryStore::purge(options.clone()),
                        )
                        .await
                        .context("busy owner maintenance refusal exceeded its maintenance deadline")?
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

                        // The permit waits out the idle owner's close within the
                        // product's maintenance deadline.
                        let permit = tokio::time::timeout(
                            maintenance_deadline(&options),
                            acquire_maintenance_permit(&options),
                        )
                        .await
                        .context(
                            "maintenance did not retire the idle owner within its maintenance deadline",
                        )??;
                        ensure!(
                            ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?
                                .is_none(),
                            "maintenance did not retain the starter election gate"
                        );
                        ensure!(
                            EndpointRecord::read(&data, &scope)?.is_none(),
                            "retired owner still published an endpoint"
                        );
                        served
                            .reap(
                                Duration::from_secs(5),
                                "retired owner did not finish reaping",
                            )
                            .await?;
                        drop(permit);
                        ensure!(
                            ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?
                                .is_some(),
                            "maintenance did not release the election gate"
                        );
                        Ok::<(), anyhow::Error>(())
                    }
                    .await
                },
                async |served| {
                    served
                        .retire(
                            &options,
                            Some(maintenance_deadline(&options)),
                            Duration::from_secs(5),
                            "retired owner did not finish reaping",
                        )
                        .await
                },
            )
            .await;
        root.release(outcome)
    }

    #[tokio::test]
    async fn retiring_endpoint_close_during_handshake_is_transient() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, closed explicitly.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
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
            let mut owner = ServiceOwner::open(options, &project).await?;
            let closed_peer = async {
                let stream = owner.accept(HANDSHAKE_TIMEOUT).await?;
                drop(stream);
                Ok::<(), anyhow::Error>(())
            };
            let (attached, closed) = tokio::join!(try_attach(&data, &scope, &project), closed_peer);
            closed?;
            ensure!(
                attached?.is_none(),
                "closed retiring handshake produced a live attachment"
            );
            owner.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("retiring-handshake fixture exceeded its {deadline:?} deadline")
        })??;
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
        stderr: File,
    ) -> kuru_platform::windows::process::NativeSpawnSpec {
        use kuru_platform::windows::process::{NativeSpawnSpec, Stdio};
        let mut command = NativeSpawnSpec::new(executable.to_owned(), project.to_owned());
        command.args = service_arguments(options, project, None);
        command.args[0] = "--internal-memory-service-held-client-fixture".into();
        command.args.push(ready.as_os_str().to_owned());
        command.args.push(release.as_os_str().to_owned());
        command.lifetime = lifetime;
        // A private file, not a pipe: the starter never blocks on it and the
        // caller reads only its bounded tail after a failure.
        command.stderr = Stdio::Handle(stderr.into());
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

    /// A contained starter publishes readiness after `attach_or_start`, which
    /// caps election and owner readiness at the startup budget, plus one final
    /// attachment (connect, handshake write and handshake read, each bounded by
    /// `HANDSHAKE_TIMEOUT`), its append call, and the child's own start.
    #[cfg(windows)]
    fn windows_starter_readiness() -> Duration {
        let startup = Duration::from_secs(
            crate::store::OpenOptions::new(PathBuf::new(), String::new())
                .config
                .startup_timeout_secs,
        );
        startup
            .saturating_add(HANDSHAKE_TIMEOUT.saturating_mul(3))
            .saturating_add(crate::store::QUERY_TIMEOUT)
            .saturating_add(crate::test_support::CHILD_START_MARGIN)
    }

    #[cfg(windows)]
    async fn windows_root_exited(
        child: &kuru_platform::windows::process::NativeChild,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !child.fixture_root_has_exited()? {
            ensure!(
                tokio::time::Instant::now() < deadline,
                "contained starter root did not exit"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// A starter that fails before `attach_or_start` exits with code 1; its
    /// cause must reach the fixture failure instead of a null stderr.
    #[cfg(windows)]
    #[tokio::test]
    async fn contained_starter_failure_reports_its_stderr() -> Result<()> {
        use kuru_platform::windows::process::Lifetime;
        let (root, project, mut options, executable) = windows_service_fixture()?;
        options.supervisor = Some(PathBuf::from("relative-supervisor.exe"));
        let ready = root.path().join("starter-ready");
        let release = root.path().join("starter-release");
        let (child_stderr, mut starter_stderr) =
            open_starter_stderr(root.path(), "starter-stderr")?;
        let _gate = crate::spawn_gate::spawning().await;
        let mut starter = windows_starter_fixture(
            &project,
            &options,
            &executable,
            &ready,
            &release,
            Lifetime::OwnedJob,
            child_stderr,
        )
        .spawn()
        .await?;
        let error = await_starter_ready(
            || starter.try_wait(),
            &ready,
            tokio::time::Instant::now() + windows_starter_readiness(),
            &mut starter_stderr,
        )
        .await
        .err()
        .context("a starter with an invalid supervisor path published readiness")?;
        let rendered = format!("{error:#}");
        ensure!(
            rendered.starts_with("contained memory starter exited before readiness: exit code: 1"),
            "starter failure lost its leading text: {rendered}"
        );
        ensure!(
            rendered.contains("contained memory starter stderr tail: ")
                && rendered.contains("service supervisor path must be absolute"),
            "starter failure discarded the child's stderr: {rendered}"
        );
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn starter_job_exit_preserves_independent_owner_and_surviving_client() -> Result<()> {
        use kuru_platform::windows::process::Lifetime;
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh spawned service owner, which retires as soon as the survivor
        // detaches.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let (root, project, options, executable) = windows_service_fixture()?;
            let ready = root.path().join("starter-ready");
            let release = root.path().join("starter-release");
            let (child_stderr, mut starter_stderr) =
                open_starter_stderr(root.path(), "starter-stderr")?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut starter = windows_starter_fixture(
                &project,
                &options,
                &executable,
                &ready,
                &release,
                Lifetime::FixtureBreakawayJob,
                child_stderr,
            )
            .spawn()
            .await?;
            let ready_deadline = tokio::time::Instant::now() + windows_starter_readiness();
            await_starter_ready(
                || starter.try_wait(),
                &ready,
                ready_deadline,
                &mut starter_stderr,
            )
            .await?;
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
            // The starter was reached before it exited, so the last detach
            // retires the owner at once; its owner lock is released only after
            // its endpoint is retired and Dolt is reaped.
            await_owner_release(&options).await?;
            ensure!(
                EndpointRecord::read(&options.data_dir, &options.project_scope)?.is_none(),
                "independent owner released its lock without retiring its endpoint"
            );
            // The owner ran in another process: record its store's quiescence
            // for the fixture root's teardown.
            crate::test_support::await_managed_quiescence(&options).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("Windows independent owner fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn denying_outer_job_contains_owner_until_close_then_recovery_succeeds() -> Result<()> {
        use kuru_platform::windows::process::Lifetime;
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: the fresh contained owner, then the recovered owner that reopens the
        // store and maintenance retires.
        let budget = crate::test_support::fixture_deadline(1, 1);
        let fixture_deadline = tokio::time::Instant::now() + budget;
        tokio::time::timeout_at(fixture_deadline, async {
            let (root, project, options, executable) = windows_service_fixture()?;
            let ready = root.path().join("contained-ready");
            let release = root.path().join("contained-release");
            let (child_stderr, mut starter_stderr) =
                open_starter_stderr(root.path(), "starter-stderr")?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut starter = windows_starter_fixture(
                &project,
                &options,
                &executable,
                &ready,
                &release,
                Lifetime::OwnedJob,
                child_stderr,
            )
            .spawn()
            .await?;
            // Leave the outer fixture time to report and reap a stalled starter.
            let ready_deadline = (tokio::time::Instant::now() + windows_starter_readiness())
                .min(fixture_deadline - Duration::from_secs(2));
            await_starter_ready(
                || starter.try_wait(),
                &ready,
                ready_deadline,
                &mut starter_stderr,
            )
            .await?;
            let generation = std::fs::read_to_string(&ready)?;
            let mut survivor = try_attach(&options.data_dir, &options.project_scope, &project)
                .await?
                .context("contained survivor could not attach to starter's owner")?;
            ensure!(survivor.generation() == generation);
            std::fs::write(&release, b"release")?;
            windows_root_exited(&starter).await?;
            ensure!(matches!(
                survivor
                    .call(ServiceCall::AppendMessage {
                        namespace: "starter-exit-fixture".into(),
                        message: kuru_core::Message::text("assistant", "survivor committed"),
                    })
                    .await?,
                ServiceValue::Unit
            ));

            // The survivor stays attached, so only the Job can end this owner:
            // closing the survivor first would retire it on its own.
            drop(starter); // closes the outer kill-on-close Job and its complete tree
            await_owner_release(&options).await?;
            ensure!(
                survivor.call(ServiceCall::Revision).await.is_err(),
                "the contained owner survived its outer Job"
            );
            survivor.close();
            let mut recovered = attach_or_start(&options, &project, &executable)
                .await
                .context("recover after outer Job terminated contained owner")?;
            ensure!(
                recovered.generation() != generation,
                "outer Job closure did not replace the contained generation"
            );
            let ServiceValue::HistoryWindow(window) = recovered
                .call(ServiceCall::HistoryWindow {
                    namespace: "starter-exit-fixture".into(),
                    limit: 4,
                })
                .await?
            else {
                bail!("recovered owner returned the wrong history result");
            };
            ensure!(
                window.total_rows == 2
                    && window.messages.len() == 2
                    && window.messages[1].plain_text() == Some("survivor committed"),
                "recovered owner lost the contained commit"
            );
            recovered.close();
            drop(acquire_maintenance_permit(&options).await?);
            // Both owners ran in other processes: record their store's
            // quiescence for the fixture root's teardown.
            crate::test_support::await_managed_quiescence(&options).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("Windows contained-owner recovery fixture exceeded its {budget:?} deadline")
        })??;
        Ok(())
    }

    fn authority() -> EndpointAuthority {
        EndpointAuthority {
            version: ProtocolVersion::CURRENT,
            project_path: b"/private/project".to_vec(),
            project_scope: "scope".into(),
            history_scope: None,
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
        ensure!(
            !is_peer_closed(&error),
            "protocol incompatibility was mistaken for a retired transport"
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
        assert!(!is_peer_closed(&error));
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

    /// A client holding the record open, as discovery does while it reads,
    /// must not turn retirement into an error on either side. On Windows a
    /// deleted name stays occupied while any handle holds it and later opens
    /// are denied (native error 5), so deleting the record in place failed
    /// both the owner's absence check and the client's next read.
    #[tokio::test]
    async fn retirement_under_a_held_reader_leaves_no_discoverable_record() -> Result<()> {
        let _gate = crate::spawn_gate::locking_async().await;
        let data = tempfile::tempdir()?;
        let scope = format!("project/{}", "c".repeat(64));
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let mut record = EndpointRecord {
            authority: authority(),
            address: "socket-retiring".into(),
        };
        record.authority.project_scope = scope.clone();
        record.publish(data.path(), &owner)?;
        let reader = crate::files::read(
            &EndpointRecord::path(data.path(), &scope)?,
            Privacy::OwnerOnly,
        )?;

        record
            .retire(data.path(), &owner)
            .context("retirement failed while a client held the record open")?;
        ensure!(
            EndpointRecord::read(data.path(), &scope)
                .context("discovery failed after retirement under a held reader")?
                .is_none(),
            "a retired record remained discoverable"
        );
        let observed =
            try_attach_observed(data.path(), &scope, Path::new("/private/project"), None)
                .await
                .context("an electing client failed on a retiring owner")?;
        ensure!(
            matches!(observed, Err(AttachMiss::NoEndpoint)),
            "an electing client did not read retirement as no endpoint"
        );
        ensure!(
            request_idle_retirement(&crate::store::OpenOptions::new(
                data.path().to_owned(),
                scope.clone(),
            ))
            .await
            .context("maintenance failed on a retiring owner")?
                == RetirementReply::NoEndpoint,
            "maintenance reached a retired owner"
        );

        drop(reader);
        let stage = EndpointRecord::directory(data.path(), &scope)?.join(RETIRED_ENDPOINT);
        ensure!(
            matches!(
                std::fs::symlink_metadata(&stage),
                Err(error) if error.kind() == io::ErrorKind::NotFound
            ),
            "the retired record's stage outlived its last reader"
        );
        drop(owner);
        Ok(())
    }

    /// A discovery read that already holds the record when the owner retires
    /// it finds no record. On Windows the held object is then delete-pending,
    /// which the checked verification refuses as denied rather than missing;
    /// only the absent discovery name shows the owner retired.
    #[test]
    fn a_read_that_meets_retirement_finds_no_record() {
        let _gate = crate::spawn_gate::locking();
        let data = tempfile::tempdir().unwrap();
        let scope = format!("project/{}", "d".repeat(64));
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
            .unwrap()
            .unwrap();
        let mut record = EndpointRecord {
            authority: authority(),
            address: "socket-retiring".into(),
        };
        record.authority.project_scope = scope.clone();
        record.publish(data.path(), &owner).unwrap();

        let mut retired = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            retired = Some(record.retire(data.path(), &owner));
        });

        retired
            .expect("the hook ran between reading and verifying the record")
            .expect("retirement failed while a discovery read held the record");
        assert!(
            read.expect("a discovery read failed only because it met retirement")
                .is_none(),
            "a discovery read that met retirement returned the retired record"
        );
    }

    /// An endpoint record published under its owner in a fresh data directory.
    fn published_endpoint() -> (tempfile::TempDir, String, ServiceLock) {
        let data = tempfile::tempdir().unwrap();
        let scope = format!("project/{}", "e".repeat(64));
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)
            .unwrap()
            .unwrap();
        let mut record = EndpointRecord {
            authority: authority(),
            address: "socket-retiring".into(),
        };
        record.authority.project_scope = scope.clone();
        record.publish(data.path(), &owner).unwrap();
        (data, scope, owner)
    }

    /// Move the held record off its discovery name to the retirement stage,
    /// as a retiring owner does, keeping its object linked there.
    fn stage_held_record(data: &Path, scope: &str) -> Result<()> {
        let record = EndpointRecord::path(data, scope)?;
        std::fs::rename(&record, record.with_file_name(RETIRED_ENDPOINT))?;
        Ok(())
    }

    /// The error of a discovery read that must still refuse what it found:
    /// the original denial, not a replaced name and not a missing record.
    fn refused_discovery(read: Result<Option<EndpointRecord>>) -> anyhow::Error {
        let error = match read {
            Ok(found) => panic!(
                "a refused record was read as {}",
                if found.is_some() {
                    "a record"
                } else {
                    "no record"
                }
            ),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "read private memory service endpoint",
            "{error:#}"
        );
        let denial = error
            .downcast_ref::<io::Error>()
            .unwrap_or_else(|| panic!("the refusal lost its filesystem error: {error:#}"));
        assert_eq!(denial.kind(), io::ErrorKind::PermissionDenied, "{error:#}");
        assert!(
            !kuru_platform::fs::is_name_replaced(denial),
            "a refused replacement was typed as a replaced name: {error:#}"
        );
        error
    }

    /// A discovery read whose held record a retiring owner stages, and a
    /// successor replaces with its own private record before the read
    /// verifies the name, finds no record; the next read finds the successor.
    #[test]
    fn a_read_that_meets_a_successor_record_finds_none_then_the_successor() {
        let _gate = crate::spawn_gate::locking();
        let (data, scope, owner) = published_endpoint();
        let mut successor = EndpointRecord {
            authority: authority(),
            address: "socket-successor".into(),
        };
        successor.authority.project_scope = scope.clone();
        successor.authority.service_generation = "successor-generation".into();

        let mut replaced = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            replaced = Some(
                stage_held_record(data.path(), &scope)
                    .and_then(|()| successor.publish(data.path(), &owner)),
            );
        });

        replaced
            .expect("the hook ran between reading and verifying the record")
            .expect("the successor failed to publish while a discovery read held the record");
        let read = match read {
            Ok(read) => read,
            Err(error) => panic!("a discovery read failed on a successor's record: {error:#}"),
        };
        assert!(
            read.is_none(),
            "a discovery read that met a successor returned the replaced record"
        );
        let next = EndpointRecord::read(data.path(), &scope)
            .unwrap()
            .expect("the next discovery read found no successor record");
        assert_eq!(next.address, successor.address);
        assert_eq!(
            next.authority.service_generation,
            successor.authority.service_generation
        );
    }

    /// A discovery read whose held record a successor's publication replaces
    /// in place, with no retirement staging it aside, before the read
    /// verifies the name finds no record on every platform: the held object
    /// is unlinked (on Windows, left delete-pending with no link). The next
    /// read finds the successor.
    #[test]
    fn a_read_that_meets_a_successor_published_over_it_finds_none_then_the_successor() {
        let _gate = crate::spawn_gate::locking();
        let (data, scope, owner) = published_endpoint();
        let mut successor = EndpointRecord {
            authority: authority(),
            address: "socket-successor".into(),
        };
        successor.authority.project_scope = scope.clone();
        successor.authority.service_generation = "successor-generation".into();

        let mut replaced = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            replaced = Some(successor.publish(data.path(), &owner));
        });

        replaced
            .expect("the hook ran between reading and verifying the record")
            .expect("the successor failed to publish over a record a discovery read held");
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                panic!("a discovery read failed on a successor published over it: {error:#}")
            }
        };
        assert!(
            read.is_none(),
            "a discovery read that met a successor returned the replaced record"
        );
        let next = EndpointRecord::read(data.path(), &scope)
            .unwrap()
            .expect("the next discovery read found no successor record");
        assert_eq!(next.address, successor.address);
        assert_eq!(
            next.authority.service_generation,
            successor.authority.service_generation
        );
    }

    /// A replacement that is not a regular file is still refused by the
    /// checked read of the name, not taken for a successor's record.
    #[test]
    fn a_read_that_meets_a_non_regular_replacement_is_refused() {
        let _gate = crate::spawn_gate::locking();
        let (data, scope, _owner) = published_endpoint();
        let path = EndpointRecord::path(data.path(), &scope).unwrap();

        let mut replaced = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            replaced = Some(
                stage_held_record(data.path(), &scope)
                    .and_then(|()| Ok(std::fs::create_dir(&path)?)),
            );
        });

        replaced
            .expect("the hook ran between reading and verifying the record")
            .expect("a directory did not replace the held record");
        refused_discovery(read);
    }

    /// A replacement that is not private, and the held record itself losing
    /// its privacy, are still refused by the checked read of the name. The
    /// non-private object uses Unix modes, as the other memory privacy
    /// fixtures do.
    #[cfg(unix)]
    #[test]
    fn a_read_that_meets_a_privacy_denial_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let _gate = crate::spawn_gate::locking();
        let shared = std::fs::Permissions::from_mode(0o644);
        let (data, scope, owner) = published_endpoint();
        let path = EndpointRecord::path(data.path(), &scope).unwrap();
        let mut successor = EndpointRecord {
            authority: authority(),
            address: "socket-successor".into(),
        };
        successor.authority.project_scope = scope.clone();

        let mut replaced = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            replaced = Some(
                stage_held_record(data.path(), &scope)
                    .and_then(|()| successor.publish(data.path(), &owner))
                    .and_then(|()| Ok(std::fs::set_permissions(&path, shared.clone())?)),
            );
        });
        replaced
            .expect("the hook ran between reading and verifying the record")
            .expect("a non-private record did not replace the held record");
        let error = refused_discovery(read);
        assert!(format!("{error:#}").contains("private object"), "{error:#}");

        let (data, scope, _owner) = published_endpoint();
        let path = EndpointRecord::path(data.path(), &scope).unwrap();
        let mut widened = None;
        let read = EndpointRecord::read_then(data.path(), &scope, || {
            widened = Some(std::fs::set_permissions(&path, shared.clone()));
        });
        widened
            .expect("the hook ran between reading and verifying the record")
            .expect("the held record's mode did not change");
        let error = refused_discovery(read);
        assert!(format!("{error:#}").contains("private object"), "{error:#}");
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
                held: None,
                retain_after_abandon: false,
                identity: AttachmentIdentity::new(),
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
    async fn owner_retires_endpoint_then_reaps_before_releasing_its_lock() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner; the second open is refused on its owner lock
        // before another engine starts.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
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
                ServiceOwner::open(options.clone(), &project).await.is_err(),
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
                owner_lock_free(&options)?,
                "closed owner retained its election lock"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("real memory service owner fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_reply_receipt_survives_sibling_write_and_owner_restart() -> Result<()> {
        use tokio::io::AsyncWriteExt;
        use uuid::Uuid;

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh owner, then its reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
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
            // A one-shot successor open; see `crate::spawn_gate::excluding_spawns`.
            let (mut successor, _gate) = crate::spawn_gate::excluding_spawns(_gate, async {
                owner.close().await?;
                ServiceOwner::open(options, &project).await
            })
            .await?;
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
        .with_context(|| {
            format!(
                "lost-reply receipt and owner restart fixture exceeded its {deadline:?} deadline"
            )
        })??;
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
            // No settlement wait: the registered, unpublished write is in flight.
            progress.set_settlement_wait(Some(Duration::ZERO));
            ensure!(
                matches!(
                    query(&mut owner, &data, &scope, &authority, &progress, outcome()).await?,
                    ServiceValue::Outcome(rpc::OutcomeStatus::InFlight)
                ),
                "registered in-flight write was not reported as in flight"
            );
            pause.release.notify_one();
            let (write_result, server_result) = tokio::time::timeout(OWNER_STEP_WITHIN, async {
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
        let closed = tokio::time::timeout(crate::server::close_budget(), owner.close())
            .await
            .context("in-flight outcome fixture owner did not reap")?;
        tested.context("in-flight outcome fixture exceeded 40 seconds")??;
        closed?;
        Ok(())
    }

    /// One real owner whose single-request and managed attachments share its
    /// own receipt progress, as the owner's attachment tasks do.
    struct SettlementFixture {
        _gate: tokio::sync::RwLockReadGuard<'static, ()>,
        _root: crate::test_support::TempDir,
        data: PathBuf,
        scope: String,
        owner: ServiceOwner,
        authority: EndpointAuthority,
        progress: std::sync::Arc<rpc::ReceiptProgress>,
    }

    /// One request on its own attachment, served by the fixture owner.
    /// Dropping it aborts both halves.
    struct Served {
        client: tokio::task::JoinHandle<Result<rpc::ServiceResponse>>,
        server: tokio::task::JoinHandle<Result<()>>,
    }

    impl Served {
        fn is_finished(&self) -> bool {
            self.client.is_finished() || self.server.is_finished()
        }

        /// One request round trip, which its client bounds by its reply
        /// deadline (`OPERATION_TIMEOUT`): a write's budget and an outcome
        /// handler's budget both fit inside it.
        async fn finish(mut self) -> Result<rpc::ServiceResponse> {
            let (client, server) = tokio::time::timeout(rpc::OPERATION_TIMEOUT, async {
                tokio::join!(&mut self.client, &mut self.server)
            })
            .await
            .context("served request did not finish")?;
            server??;
            client?
        }
    }

    impl Drop for Served {
        fn drop(&mut self) {
            self.client.abort();
            self.server.abort();
        }
    }

    impl SettlementFixture {
        async fn open() -> Result<Self> {
            crate::test_support::warm_runtime_cache().await?;
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
            let options =
                crate::test_support::warmed_open_options(data.clone(), scope.clone()).await?;
            let gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options, &project).await?;
            let authority = owner.authority().clone();
            let progress = owner.receipt_progress.clone();
            Ok(Self {
                _gate: gate,
                _root: root,
                data,
                scope,
                owner,
                authority,
                progress,
            })
        }

        async fn pair(&mut self) -> Result<(LocalStream, LocalStream)> {
            let client = connect_local(
                &self.data,
                &self.scope,
                &self.owner.record.address,
                HANDSHAKE_TIMEOUT,
            )
            .await?;
            let server = self.owner.accept(HANDSHAKE_TIMEOUT).await?;
            Ok((client, server))
        }

        /// Serve one request on a fresh single-request attachment.
        async fn spawn_one(&mut self, id: uuid::Uuid, call: ServiceCall) -> Result<Served> {
            let (mut client, mut server) = self.pair().await?;
            let authority = self.authority.clone();
            let client = tokio::spawn(async move {
                connect_handshake(&mut client, &authority).await?;
                rpc::exchange_attached_with_id(&mut client, &authority, id, call).await
            });
            let authority = self.authority.clone();
            let store = self.owner.store.clone();
            let progress = self.progress.clone();
            let server = tokio::spawn(async move {
                rpc::serve_one_with_progress(&mut server, &authority, &store, &progress).await
            });
            Ok(Served { client, server })
        }

        async fn call(&mut self, call: ServiceCall) -> Result<rpc::ServiceResponse> {
            self.spawn_one(uuid::Uuid::new_v4(), call)
                .await?
                .finish()
                .await
        }

        /// A managed attachment counted by `retirement`, as `attach` counts
        /// one, and its authenticated client stream.
        async fn attach_managed(
            &mut self,
            budget: &std::sync::Arc<tokio::sync::Semaphore>,
            retirement: &std::sync::Arc<rpc::Retirement>,
        ) -> Result<(LocalStream, tokio::task::JoinHandle<Result<()>>)> {
            let (mut client, mut server) = self.pair().await?;
            let retained = retirement
                .attached()
                .context("test retirement refused an attachment")?;
            let authority = self.authority.clone();
            let store = self.owner.store.clone();
            let budget = budget.clone();
            let retirement = retirement.clone();
            let progress = self.progress.clone();
            let served = tokio::spawn(async move {
                let _retained = retained;
                rpc::serve_attached(
                    &mut server,
                    &authority,
                    &store,
                    budget,
                    retirement,
                    progress,
                )
                .await
            });
            connect_handshake(&mut client, &self.authority).await?;
            Ok((client, served))
        }

        async fn close(self) -> Result<()> {
            tokio::time::timeout(crate::server::close_budget(), self.owner.close())
                .await
                .context("settlement fixture owner did not reap")?
        }
    }

    /// Builds the main-view outcome query for `write` sent with `id`.
    fn unit_outcome(
        authority: &EndpointAuthority,
        id: uuid::Uuid,
        write: &ServiceCall,
    ) -> Result<impl Fn() -> ServiceCall + Send + 'static> {
        let (method, argument_digest) = write
            .unit_receipt_fingerprint("main")?
            .context("write has no logical receipt")?;
        let generation = authority.service_generation.clone();
        Ok(move || ServiceCall::Outcome {
            original_id: id,
            original_generation: generation.clone(),
            view: "main".into(),
            method: method.into(),
            argument_digest: argument_digest.clone(),
        })
    }

    fn outcome_status(response: rpc::ServiceResponse) -> Result<rpc::OutcomeStatus> {
        match rpc::resolve_response(response)? {
            ServiceValue::Outcome(status) => Ok(status),
            other => bail!("outcome query returned {other:?}"),
        }
    }

    /// Settlement wait events come from an outcome handler, which answers
    /// within its handler budget of its entry, so every event precedes its
    /// client's reply deadline (`OPERATION_TIMEOUT`).
    async fn next_wait_event(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<rpc::WaitEvent>,
    ) -> Result<rpc::WaitEvent> {
        tokio::time::timeout(rpc::OPERATION_TIMEOUT, events.recv())
            .await
            .context("no settlement wait event within the reply deadline")?
            .context("settlement wait events closed")
    }

    fn append(text: &str) -> ServiceCall {
        ServiceCall::AppendMessage {
            namespace: "settlement".into(),
            message: kuru_core::Message::text("user", text),
        }
    }

    async fn settled_fixture<F>(test: F) -> Result<()>
    where
        F: AsyncFnOnce(&mut SettlementFixture) -> Result<()>,
    {
        let mut fixture = SettlementFixture::open().await?;
        let tested = tokio::time::timeout(Duration::from_secs(90), test(&mut fixture)).await;
        let closed = fixture.close().await;
        tested.context("settlement fixture exceeded 90 seconds")??;
        closed
    }

    /// T1: durable evidence answers at once while the writer is registered.
    #[tokio::test]
    async fn committed_receipt_is_reported_while_its_writer_is_registered() -> Result<()> {
        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let pause = std::sync::Arc::new(rpc::SettlementPause::default());
            fx.progress.pause_settlement_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let write = append("published before settlement");
            let outcome = unit_outcome(&fx.authority, id, &write)?;
            let writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not reach settlement")?;
            let reported = outcome_status(fx.call(outcome()).await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Committed,
                "visible receipt of a registered write was reported {reported:?}"
            );
            ensure!(!writer.is_finished(), "writer finished before release");
            ensure!(
                events.try_recv().is_err(),
                "a visible receipt still waited for settlement"
            );
            pause.release.notify_one();
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));
            let settled = outcome_status(fx.call(outcome()).await?)?;
            ensure!(
                settled == rpc::OutcomeStatus::Committed,
                "settled write reported {settled:?}"
            );
            Ok(())
        })
        .await
    }

    /// T2: before the receipt exists, the query waits and then answers.
    #[tokio::test]
    async fn unpublished_receipt_waits_for_settlement_then_commits() -> Result<()> {
        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
            fx.progress.pause_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let write = append("published after the query");
            let outcome = unit_outcome(&fx.authority, id, &write)?;
            let writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not register")?;
            let query = fx.spawn_one(uuid::Uuid::new_v4(), outcome()).await?;
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            ensure!(!query.is_finished(), "query answered before settlement");
            pause.release.notify_one();
            let reported = outcome_status(query.finish().await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Committed,
                "query reported {reported:?}"
            );
            ensure!(
                next_wait_event(&mut events).await?
                    == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Settled)
            );
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));
            Ok(())
        })
        .await
    }

    /// T4: a rejected receipt-bearing write is absent only after it settles;
    /// with no wait budget it is still in flight.
    #[tokio::test]
    async fn rejected_write_is_absent_only_after_settlement() -> Result<()> {
        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
            fx.progress.pause_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            // Rejected by validation before any SQL write.
            let write = ServiceCall::AppendMessage {
                namespace: " ".into(),
                message: kuru_core::Message::text("user", "never stored"),
            };
            let outcome = unit_outcome(&fx.authority, id, &write)?;
            let writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not register")?;
            fx.progress.set_settlement_wait(Some(Duration::ZERO));
            let unbudgeted = outcome_status(fx.call(outcome()).await?)?;
            ensure!(
                unbudgeted == rpc::OutcomeStatus::InFlight,
                "zero-budget query reported {unbudgeted:?}"
            );
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            ensure!(
                next_wait_event(&mut events).await?
                    == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Exhausted)
            );
            fx.progress.set_settlement_wait(None);
            let query = fx.spawn_one(uuid::Uuid::new_v4(), outcome()).await?;
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            ensure!(!query.is_finished(), "query answered before settlement");
            pause.release.notify_one();
            let reported = outcome_status(query.finish().await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Absent,
                "query reported {reported:?}"
            );
            ensure!(
                next_wait_event(&mut events).await?
                    == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Settled)
            );
            ensure!(rpc::resolve_response(writer.finish().await?).is_err());
            Ok(())
        })
        .await
    }

    /// T5: usage-ledger proofs answer from the probe or after settlement.
    #[tokio::test]
    async fn usage_proof_is_committed_while_registered_and_after_settlement() -> Result<()> {
        use kuru_core::{InvocationStart, UsagePhase};

        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let start = |invocation: &str| InvocationStart {
                session_id: "settlement-session".into(),
                invocation_id: invocation.into(),
                operation_id: "settlement-turn".into(),
                phase: UsagePhase::Speak,
                actor_id: "speaker".into(),
                route: "responses".into(),
                model: "model".into(),
                price_at_invocation: None,
            };
            fx.owner
                .store
                .usage_ledger()?
                .mark_new_session("settlement-session")
                .await?;
            let admit = |invocation: &str| ServiceCall::Ledger {
                operation: Box::new(rpc::LedgerOperation::Admit {
                    start: Box::new(start(invocation)),
                }),
            };
            let query = |fx: &SettlementFixture, id, invocation: &str| -> Result<ServiceCall> {
                Ok(ServiceCall::LedgerOutcome {
                    original_id: id,
                    original_generation: fx.authority.service_generation.clone(),
                    proof: crate::store::UsageProof::admit(&start(invocation))?,
                })
            };

            let settlement = std::sync::Arc::new(rpc::SettlementPause::default());
            fx.progress.pause_settlement_next(settlement.clone());
            let id = uuid::Uuid::new_v4();
            let writer = fx.spawn_one(id, admit("visible")).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, settlement.entered.notified())
                .await
                .context("usage write did not reach settlement")?;
            let reported = outcome_status(fx.call(query(fx, id, "visible")?).await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Committed,
                "visible proof reported {reported:?}"
            );
            ensure!(events.try_recv().is_err(), "a visible proof still waited");
            settlement.release.notify_one();
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));

            let registered = std::sync::Arc::new(rpc::RegisteredPause::default());
            fx.progress.pause_next(registered.clone());
            let id = uuid::Uuid::new_v4();
            let writer = fx.spawn_one(id, admit("waited")).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, registered.entered.notified())
                .await
                .context("usage write did not register")?;
            let waiting = fx
                .spawn_one(uuid::Uuid::new_v4(), query(fx, id, "waited")?)
                .await?;
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            ensure!(
                !waiting.is_finished(),
                "usage query answered before settlement"
            );
            registered.release.notify_one();
            let reported = outcome_status(waiting.finish().await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Committed,
                "waited proof reported {reported:?}"
            );
            ensure!(
                next_wait_event(&mut events).await?
                    == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Settled)
            );
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));
            Ok(())
        })
        .await
    }

    /// T6: candidate transitions and selected abandonment are read under the
    /// guard only after settlement; with no wait budget they are in flight.
    #[tokio::test]
    async fn candidate_transitions_answer_only_after_settlement() -> Result<()> {
        use rpc::{CandidateTransitionKind, CandidateTransitionResult, ViewOperation};

        async fn request(
            client: &mut LocalStream,
            authority: &EndpointAuthority,
            call: ServiceCall,
        ) -> Result<ServiceValue> {
            rpc::resolve_response(
                rpc::exchange_attached_with_id(client, authority, uuid::Uuid::new_v4(), call)
                    .await?,
            )
        }

        fn transition(response: rpc::ServiceResponse) -> Result<CandidateTransitionResult> {
            match rpc::resolve_response(response)? {
                ServiceValue::CandidateTransitionOutcome(result) => Ok(result),
                other => bail!("transition query returned {other:?}"),
            }
        }

        /// Pause `call` at settlement, show a zero-budget query in flight,
        /// then show a budgeted query waiting until release.
        async fn paused_transition(
            fx: &mut SettlementFixture,
            events: &mut tokio::sync::mpsc::UnboundedReceiver<rpc::WaitEvent>,
            mut client: LocalStream,
            id: uuid::Uuid,
            call: ServiceCall,
            query: impl Fn() -> ServiceCall,
        ) -> Result<(rpc::ServiceResponse, CandidateTransitionResult)> {
            let pause = std::sync::Arc::new(rpc::SettlementPause::default());
            fx.progress.pause_settlement_next(pause.clone());
            let authority = fx.authority.clone();
            let mut writer = tokio::spawn(async move {
                rpc::exchange_attached_with_id(&mut client, &authority, id, call).await
            });
            let observed = async {
                tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                    .await
                    .context("transition did not reach settlement")?;
                fx.progress.set_settlement_wait(Some(Duration::ZERO));
                let unbudgeted = transition(fx.call(query()).await?)?;
                ensure!(
                    matches!(unbudgeted, CandidateTransitionResult::InFlight),
                    "zero-budget transition query reported {unbudgeted:?}"
                );
                ensure!(next_wait_event(events).await? == rpc::WaitEvent::Entered);
                ensure!(
                    next_wait_event(events).await?
                        == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Exhausted)
                );
                fx.progress.set_settlement_wait(None);
                let waiting = fx.spawn_one(uuid::Uuid::new_v4(), query()).await?;
                ensure!(next_wait_event(events).await? == rpc::WaitEvent::Entered);
                ensure!(
                    !waiting.is_finished(),
                    "transition query answered before settlement"
                );
                pause.release.notify_one();
                let answered = transition(waiting.finish().await?)?;
                ensure!(
                    next_wait_event(events).await?
                        == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::Settled)
                );
                let written = tokio::time::timeout(Duration::from_secs(20), &mut writer)
                    .await
                    .context("transition writer did not finish")???;
                Ok((written, answered))
            }
            .await;
            pause.release.notify_one();
            writer.abort();
            observed
        }

        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let budget = std::sync::Arc::new(tokio::sync::Semaphore::new(rpc::FRAME_BUDGET_MIB));
            let retirement = std::sync::Arc::new(rpc::Retirement::default());
            for kind in [CandidateTransitionKind::Promote, CandidateTransitionKind::Abandon] {
                let (mut client, served) = fx.attach_managed(&budget, &retirement).await?;
                let ServiceValue::CandidateStarted { handle, base, branch } = request(
                    &mut client,
                    &fx.authority,
                    ServiceCall::BeginCandidate { label: "settled transition".into() },
                )
                .await?
                else {
                    bail!("service did not start the candidate")
                };
                ensure!(matches!(
                    request(
                        &mut client,
                        &fx.authority,
                        ServiceCall::View {
                            candidate: Some(handle),
                            operation: Box::new(ViewOperation::PutMany {
                                values: vec![("private".into(), serde_json::json!(1))],
                            }),
                        },
                    )
                    .await?,
                    ServiceValue::Unit
                ));
                let ServiceValue::Revision(target) = request(
                    &mut client,
                    &fx.authority,
                    ServiceCall::View {
                        candidate: Some(handle),
                        operation: Box::new(ViewOperation::Revision),
                    },
                )
                .await?
                else {
                    bail!("service did not report the candidate target")
                };
                let id = uuid::Uuid::new_v4();
                let call = match kind {
                    CandidateTransitionKind::Promote => ServiceCall::PromoteCandidate {
                        handle,
                        branch: branch.clone(),
                        base: base.clone(),
                        target: target.clone(),
                    },
                    CandidateTransitionKind::Abandon => ServiceCall::AbandonCandidate {
                        handle,
                        branch: branch.clone(),
                        base: base.clone(),
                        target: target.clone(),
                    },
                };
                let generation = fx.authority.service_generation.clone();
                let expected = target.clone();
                let query = move || ServiceCall::CandidateTransitionOutcome {
                    original_id: id,
                    original_generation: generation.clone(),
                    transition: kind,
                    branch: branch.clone(),
                    base: base.clone(),
                    target: expected.clone(),
                };
                let (written, answered) =
                    paused_transition(fx, &mut events, client, id, call, query).await?;
                match kind {
                    CandidateTransitionKind::Promote => {
                        ensure!(matches!(
                            rpc::resolve_response(written)?,
                            ServiceValue::Revision(revision) if revision == target
                        ));
                        ensure!(
                            matches!(&answered, CandidateTransitionResult::Promoted { revision } if *revision == target),
                            "settled promotion reported {answered:?}"
                        );
                    }
                    CandidateTransitionKind::Abandon => {
                        ensure!(matches!(rpc::resolve_response(written)?, ServiceValue::Unit));
                        ensure!(
                            matches!(answered, CandidateTransitionResult::Abandoned),
                            "settled abandonment reported {answered:?}"
                        );
                    }
                }
                tokio::time::timeout(OWNER_STEP_WITHIN, served)
                    .await
                    .context("transition attachment did not end")???;
            }

            // Selected abandonment needs a managed owner with no other
            // counted attachment; the queries are unmanaged.
            let candidate = fx.owner.store.begin_candidate("selected abandonment").await?;
            let branch = candidate.view().pinned_view().to_owned();
            let base = candidate.base().to_owned();
            candidate.view().put("private", &serde_json::json!(2)).await?;
            let target = candidate.view().revision().await?;
            drop(candidate);
            let (client, served) = fx.attach_managed(&budget, &retirement).await?;
            let id = uuid::Uuid::new_v4();
            let (written, answered) = paused_transition(
                fx,
                &mut events,
                client,
                id,
                ServiceCall::AbandonCandidateRef {
                    branch: branch.clone(),
                    base: base.clone(),
                    target: target.clone(),
                },
                {
                    let generation = fx.authority.service_generation.clone();
                    move || ServiceCall::SelectedAbandonOutcome {
                        original_id: id,
                        original_generation: generation.clone(),
                        branch: branch.clone(),
                        base: base.clone(),
                        target: target.clone(),
                    }
                },
            )
            .await?;
            ensure!(matches!(rpc::resolve_response(written)?, ServiceValue::Unit));
            ensure!(
                matches!(answered, CandidateTransitionResult::Abandoned),
                "settled selected abandonment reported {answered:?}"
            );
            tokio::time::timeout(OWNER_STEP_WITHIN, served)
                .await
                .context("selected abandonment attachment did not end")???;
            Ok(())
        })
        .await
    }

    /// T7: candidate creation is read under the guard after settlement.
    #[tokio::test]
    async fn candidate_creation_answers_open_after_settlement() -> Result<()> {
        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let pause = std::sync::Arc::new(rpc::SettlementPause::default());
            fx.progress.pause_settlement_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let writer = fx
                .spawn_one(
                    id,
                    ServiceCall::BeginCandidate {
                        label: "settled creation".into(),
                    },
                )
                .await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("candidate creation did not reach settlement")?;
            let query = fx
                .spawn_one(
                    uuid::Uuid::new_v4(),
                    ServiceCall::CandidateOutcome {
                        original_id: id,
                        original_generation: fx.authority.service_generation.clone(),
                    },
                )
                .await?;
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            ensure!(
                !query.is_finished(),
                "creation query answered before settlement"
            );
            pause.release.notify_one();
            let ServiceValue::CandidateOutcome(rpc::CandidateCreationOutcome::Open {
                base: observed_base,
                branch: observed_branch,
                ..
            }) = rpc::resolve_response(query.finish().await?)?
            else {
                bail!("settled candidate creation was not reported open")
            };
            let ServiceValue::CandidateStarted { base, branch, .. } =
                rpc::resolve_response(writer.finish().await?)?
            else {
                bail!("candidate creation did not start")
            };
            ensure!(observed_base == base && observed_branch == branch);
            Ok(())
        })
        .await
    }

    /// T10: a handler dropped before it returned never proves absence.
    #[tokio::test]
    async fn dropped_registered_write_is_not_reported_absent() -> Result<()> {
        settled_fixture(async |fx| {
            let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
            fx.progress.pause_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let write = append("dropped before dispatch");
            let outcome = unit_outcome(&fx.authority, id, &write)?;
            let mut writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not register")?;
            writer.server.abort();
            let aborted = tokio::time::timeout(OWNER_STEP_WITHIN, &mut writer.server)
                .await
                .context("aborted serve task did not end")?;
            ensure!(aborted.is_err_and(|error| error.is_cancelled()));
            drop(writer);
            let reported = outcome_status(fx.call(outcome()).await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::StillUncertain,
                "dropped handler was reported {reported:?}"
            );
            Ok(())
        })
        .await
    }

    /// T11: the lock-free probe keeps the receipt-conflict fault.
    #[tokio::test]
    async fn probe_reports_a_conflicting_receipt_as_a_fault() -> Result<()> {
        settled_fixture(async |fx| {
            let pause = std::sync::Arc::new(rpc::SettlementPause::default());
            fx.progress.pause_settlement_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let write = append("the original arguments");
            let conflicting = unit_outcome(&fx.authority, id, &append("different arguments"))?;
            let writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not reach settlement")?;
            let response = fx.call(conflicting()).await?;
            ensure!(
                matches!(
                    response,
                    rpc::ServiceResponse::Rejected(rpc::ServiceFault::ReceiptConflict)
                ),
                "conflicting receipt was reported {response:?}"
            );
            pause.release.notify_one();
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));
            Ok(())
        })
        .await
    }

    /// T12 (lead condition 1): a waiting outcome query holds no write guard;
    /// other attachments read and write while it waits.
    #[tokio::test]
    async fn outcome_wait_leaves_other_attachments_free() -> Result<()> {
        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
            fx.progress.pause_next(pause.clone());
            let id = uuid::Uuid::new_v4();
            let write = append("paused writer");
            let outcome = unit_outcome(&fx.authority, id, &write)?;
            let writer = fx.spawn_one(id, write).await?;
            tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                .await
                .context("write did not register")?;
            let query = fx.spawn_one(uuid::Uuid::new_v4(), outcome()).await?;
            ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
            let read = fx
                .call(ServiceCall::Get {
                    key: "absent".into(),
                })
                .await?;
            ensure!(matches!(
                rpc::resolve_response(read)?,
                ServiceValue::StoredValue(None)
            ));
            let written = fx.call(append("another session's write")).await?;
            ensure!(matches!(
                rpc::resolve_response(written)?,
                ServiceValue::Unit
            ));
            ensure!(!query.is_finished(), "query answered before settlement");
            ensure!(!writer.is_finished(), "paused writer finished");
            pause.release.notify_one();
            let reported = outcome_status(query.finish().await?)?;
            ensure!(
                reported == rpc::OutcomeStatus::Committed,
                "query reported {reported:?}"
            );
            ensure!(matches!(
                rpc::resolve_response(writer.finish().await?)?,
                ServiceValue::Unit
            ));
            Ok(())
        })
        .await
    }

    /// T13 (lead condition 1): when the querying client disconnects or is
    /// cancelled, the owner's wait ends while the writer is still paused, and
    /// the attachment's frame budget, retirement count and waiter are released.
    #[tokio::test]
    async fn outcome_wait_ends_when_its_client_leaves() -> Result<()> {
        use tokio::io::AsyncWriteExt;

        settled_fixture(async |fx| {
            let mut events = fx.progress.watch_waits();
            let budget = std::sync::Arc::new(tokio::sync::Semaphore::new(rpc::FRAME_BUDGET_MIB));
            let retirement = std::sync::Arc::new(rpc::Retirement::default());
            for cancel in [false, true] {
                let pause = std::sync::Arc::new(rpc::RegisteredPause::default());
                fx.progress.pause_next(pause.clone());
                let id = uuid::Uuid::new_v4();
                let write = append(if cancel {
                    "cancelled query"
                } else {
                    "disconnected query"
                });
                let outcome = unit_outcome(&fx.authority, id, &write)?;
                let writer = fx.spawn_one(id, write).await?;
                tokio::time::timeout(OWNER_STEP_WITHIN, pause.entered.notified())
                    .await
                    .context("write did not register")?;
                ensure!(retirement.active_for_test() == 0);
                let (mut client, served) = fx.attach_managed(&budget, &retirement).await?;
                ensure!(retirement.active_for_test() == 1);
                let client = if cancel {
                    let authority = fx.authority.clone();
                    let query = outcome();
                    let call = tokio::spawn(async move {
                        rpc::exchange_attached_with_id(
                            &mut client,
                            &authority,
                            uuid::Uuid::new_v4(),
                            query,
                        )
                        .await
                    });
                    ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
                    call.abort();
                    let cancelled = tokio::time::timeout(OWNER_STEP_WITHIN, call)
                        .await
                        .context("cancelled client call did not end")?;
                    ensure!(cancelled.is_err_and(|error| error.is_cancelled()));
                    None
                } else {
                    let request =
                        rpc::ServiceRequest::new(&fx.authority.service_generation, outcome());
                    let body = serde_json::to_vec(&request)?;
                    client.write_all(&(body.len() as u32).to_be_bytes()).await?;
                    client.write_all(&body).await?;
                    client.flush().await?;
                    ensure!(next_wait_event(&mut events).await? == rpc::WaitEvent::Entered);
                    Some(client)
                };
                drop(client);
                ensure!(
                    next_wait_event(&mut events).await?
                        == rpc::WaitEvent::Ended(rpc::SettlementWaitEnd::ClientGone),
                    "query wait did not end on client departure"
                );
                ensure!(
                    !writer.is_finished(),
                    "writer settled before the client left"
                );
                let ended = tokio::time::timeout(OWNER_STEP_WITHIN, served)
                    .await
                    .context("query attachment did not end")??;
                if let Err(error) = ended {
                    ensure!(is_peer_closed(&error), "query attachment failed: {error:#}");
                }
                ensure!(budget.available_permits() == rpc::FRAME_BUDGET_MIB);
                ensure!(retirement.active_for_test() == 0);
                ensure!(fx.progress.waiters_for_test() == 0);
                pause.release.notify_one();
                ensure!(matches!(
                    rpc::resolve_response(writer.finish().await?)?,
                    ServiceValue::Unit
                ));
                let reported = outcome_status(fx.call(outcome()).await?)?;
                ensure!(
                    reported == rpc::OutcomeStatus::Committed,
                    "fresh query reported {reported:?}"
                );
            }
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn crashed_owner_retains_accepted_receipt_after_sibling_write() -> Result<()> {
        let (_, outcome) = crashed_owner_receipt_fixture(None).await?;
        outcome
    }

    /// The failure the crash fixture's early-exit test injects.
    const CRASH_FIXTURE_INJECTED: &str = "injected crash-fixture failure while its owner serves";

    /// A crash fixture that fails while its spawned owner and that owner's
    /// Dolt are running reports its own error, after the killed owner's
    /// supervisor has reaped Dolt: the root is released, not kept, and no
    /// teardown error or guard verdict is attached to the injected error.
    #[tokio::test]
    async fn crashed_owner_fixture_failure_is_reported_after_its_engine_quiesces() -> Result<()> {
        let (root, outcome) = crashed_owner_receipt_fixture(Some(CRASH_FIXTURE_INJECTED)).await?;
        let error = outcome
            .err()
            .context("the injected failure was not reported")?;
        ensure!(
            format!("{error:#}") == CRASH_FIXTURE_INJECTED,
            "the fixture reported {error:#}"
        );
        ensure!(!root.exists(), "fixture root {} was kept", root.display());
        Ok(())
    }

    /// A spawned owner accepts a write whose reply is lost, a sibling writes,
    /// the owner crashes, and the elected successor recovers the accepted
    /// receipt. Returns the fixture root's path and the fixture's outcome
    /// after the root's release.
    ///
    /// The root and options live outside the timed stage. On every exit path
    /// (success, a body error, `inject` failing the body while the owner
    /// serves, or an elapsed deadline, which drops the stage and with it
    /// `KillServiceOnDrop`), the fixture then awaits managed quiescence, so
    /// the killed owner's supervisor has reaped Dolt (or a successor has
    /// retired) before the root is released. A body error is returned first.
    async fn crashed_owner_receipt_fixture(
        inject: Option<&'static str>,
    ) -> Result<(PathBuf, Result<()>)> {
        use tokio::io::AsyncWriteExt;

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: the fresh crashed spawned owner, then the elected successor that reopens
        // the store and maintenance retires.
        let budget = crate::test_support::fixture_deadline(1, 1);
        let deadline =
            crate::test_support::FixtureDeadline::start(budget, "crashed-owner receipt fixture");
        let fixture_deadline = tokio::time::Instant::now() + budget;
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
        let stage = deadline.run(async {
            let _gate = crate::spawn_gate::spawning().await;
            let diagnostic_path = root.path().join("crash-service.stderr");
            let mut diagnostic = File::options()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&diagnostic_path)?;
            // A token no attachment here presents: this fixture is never the
            // owner's starter, so the owner does not retire when the original
            // attachment drops before the sibling attaches (within its startup
            // budget); only the deliberate crash below ends it.
            let mut process = KillServiceOnDrop(ServiceProcess::new(
                spawn_service(
                    &options,
                    &project,
                    &executable,
                    Some(diagnostic.try_clone()?),
                    Some(uuid::Uuid::new_v4()),
                )
                .await?,
            ));
            // The spawned owner creates its store with one fresh open. Preserve
            // the existing diagnostic and process cleanup path on expiry.
            let deadline = (tokio::time::Instant::now()
                + crate::test_support::fresh_open_budget()
                + crate::test_support::CHILD_START_MARGIN)
                .min(fixture_deadline - Duration::from_secs(2));
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
            // The owner commits the accepted write within its write budget
            // (`QUERY_TIMEOUT`, taken by `write_deadline`).
            tokio::time::timeout(crate::store::QUERY_TIMEOUT, async {
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
            ensure!(
                process.0.try_wait()?.is_none(),
                "fixture owner exited before the deliberate crash"
            );
            if let Some(failure) = inject {
                bail!(failure);
            }
            process.terminate()?;
            drop(sibling);
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
        });
        // Both owners ran in other processes. On every exit path, record
        // their store's quiescence before the guarded root is released: the
        // successor retires, or the killed owner's supervisor reaps Dolt.
        let outcome = crate::test_support::settle(
            stage.await,
            crate::test_support::await_managed_quiescence(&options),
        )
        .await;
        let path = root.path().to_path_buf();
        Ok((path, root.release(outcome)))
    }

    #[tokio::test]
    async fn lost_usage_reply_uses_natural_key_after_sibling_write_and_restart() -> Result<()> {
        use kuru_core::{InvocationStart, UsagePhase};
        use tokio::io::AsyncWriteExt;
        use uuid::Uuid;

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh owner, then its reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
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
            let options =
                crate::test_support::warmed_open_options(data.clone(), scope.clone()).await?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project)
                .await
                .context("initial usage fixture owner could not open")?;
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
            // A sibling test may spawn while a shared spawn guard is held. On
            // Unix, that child can briefly inherit the old owner's flock after
            // close, so exclude other test spawns through successor admission.
            drop(_gate);
            let _restart_gate = crate::spawn_gate::locking_async().await;
            owner
                .close()
                .await
                .context("usage fixture owner could not close")?;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "closed usage fixture owner retained its endpoint"
            );
            let released = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
                .context("closed usage fixture owner retained its lock")?;
            drop(released);
            let mut successor = ServiceOwner::open(options, &project)
                .await
                .context("successor usage fixture owner could not open")?;
            drop(_restart_gate);
            let _gate = crate::spawn_gate::spawning().await;
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
        .with_context(|| {
            format!("lost usage reply fixture exceeded its {deadline:?} deadline")
        })??;
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

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh owner, then its reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!("project/{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
            let data = root.path().join("private");
            let options = crate::test_support::warmed_open_options(data.clone(), scope.clone()).await?;
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
            // A one-shot successor open; see `crate::spawn_gate::excluding_spawns`.
            let (mut successor, _gate) = crate::spawn_gate::excluding_spawns(_gate, async {
                owner.close().await?;
                ServiceOwner::open(options, &project).await
            })
            .await?;
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
        .with_context(|| format!("candidate transition outcome fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_candidate_transition_replies_survive_sibling_write_and_owner_restart()
    -> Result<()> {
        use crate::service::rpc::{
            CandidateTransitionKind, CandidateTransitionResult, ViewOperation,
        };

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh owner, then its reopened successor.
        let deadline = crate::test_support::FixtureDeadline::start(
            crate::test_support::fixture_deadline(1, 1),
            "lost candidate promotion fixture",
        );
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
        let options =
            crate::test_support::warmed_open_options(root.path().join("private"), scope).await?;
        let outcome = deadline.serve(async |served| {
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            served.serve(owner)?;
            async {
                let mut client = attach_existing(&options, &project).await?.context("missing service attachment")?;
                let generation = client.generation().to_owned();
                let ServiceValue::CandidateStarted { handle, base, branch } = client
                    .call(ServiceCall::BeginCandidate { label: "lost promotion".into() })
                    .await? else { bail!("service did not start the candidate") };
                ensure!(matches!(client.call(ServiceCall::View {
                    candidate: Some(handle),
                    operation: Box::new(ViewOperation::PutMany { values: vec![("private".into(), serde_json::json!(1))] }),
                }).await?, ServiceValue::Unit));
                let ServiceValue::Revision(target) = client.call(ServiceCall::View {
                    candidate: Some(handle), operation: Box::new(ViewOperation::Revision),
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
                // No inner bound: each query answers within its handler budget,
                // but an accepted promotion is a multi-step candidate operation
                // with no single product budget and may answer `InFlight` more
                // than once, so the enclosing `FixtureDeadline` bounds the poll.
                async {
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
                }.await.context("accepted promotion proof")?;
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
                    operation: Box::new(ViewOperation::PutMany {
                        values: vec![("abandoned-private".into(), serde_json::json!(1))],
                    }),
                }).await?, ServiceValue::Unit));
                let ServiceValue::Revision(abandon_target) = abandoner.call(ServiceCall::View {
                    candidate: Some(abandon_handle), operation: Box::new(ViewOperation::Revision),
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
                // No inner bound, as for the promotion proof above.
                async {
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
                }.await.context("accepted abandonment proof")?;
                let _gate = served.restart(_gate, &options, &project, Duration::from_secs(10), "promotion fixture owner did not reap").await?;
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
                Ok::<(), anyhow::Error>(())
            }.await
        }, async |served| {
            served.retire(
                &options,
                None,
                Duration::from_secs(10),
                "transition fixture's current owner did not reap",
            ).await
        }).await;
        root.release(outcome)
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

        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: a fresh owner, then a reopened successor, local abandonment open and
        // final owner.
        let deadline = crate::test_support::fixture_deadline(1, 3);
        tokio::time::timeout(deadline, async {
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
            let options =
                crate::test_support::warmed_open_options(data.clone(), scope.clone()).await?;
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
            // A one-shot successor open; see `crate::spawn_gate::excluding_spawns`.
            let (mut successor, _gate) = crate::spawn_gate::excluding_spawns(_gate, async {
                owner.close().await?;
                ServiceOwner::open(options.clone(), &project).await
            })
            .await?;
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
            // The owner lock released by this close is next taken by the final
            // one-shot open; see `crate::spawn_gate::excluding_spawns`.
            let (mut final_owner, _gate) = crate::spawn_gate::excluding_spawns(_gate, async {
                successor.close().await?;

                let local = crate::store::MemoryStore::open(options.clone()).await?;
                let CandidateLookup::Open(candidate) = local.candidate_for_id(id).await? else {
                    bail!("exact candidate was missing before explicit abandonment")
                };
                candidate.abandon().await?;
                drop(candidate);
                local.close().await?;
                ServiceOwner::open(options, &project).await
            })
            .await?;
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
        .with_context(|| {
            format!("candidate lost-reply outcome fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn independent_clients_elect_one_real_process_and_retire_after_both_detach() -> Result<()>
    {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh elected service process, which retires as soon as both
        // clients have detached.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = crate::store::test_supervisor()?;
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
            // Awaited on the owner lock itself: it is released only after the
            // endpoint is retired and Dolt is reaped, with no idle interval.
            let detached = tokio::time::Instant::now();
            await_owner_release(&options).await?;
            let closed = detached.elapsed();
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "retired service released its owner lock without retiring its endpoint"
            );
            eprintln!(
                "memory owner close after last detach: {} ms",
                closed.as_millis()
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("multiprocess memory service fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn rejected_publication_reaps_engine_before_owner_lock_releases() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: the rejected publication's fresh open, which reaps Dolt, then the
        // reopened owner.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
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
            // The rejected open releases the owner lock that the one-shot reopen
            // takes again; see `crate::spawn_gate::excluding_spawns`.
            let ((), _gate) = crate::spawn_gate::excluding_spawns(_gate, async {
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
                reopened.close().await
            })
            .await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("publication cleanup fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn idle_retention_reuses_owner_resets_after_detach_and_preserves_active_client()
    -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        let deadline = crate::test_support::FixtureDeadline::start(
            crate::test_support::fixture_deadline(1, 0),
            "retained owner interval fixture",
        );
        let root = crate::test_support::tempdir()?;
        let (project, scope, data, options) = owner_fixture(root.path())?;
        let idle = Duration::from_millis(400);
        let outcome = deadline
            .serve(
                async |served| {
                    let _gate = crate::spawn_gate::spawning().await;
                    let owner = ServiceOwner::open(options.clone(), &project).await?;
                    let authority = owner.authority().clone();
                    let (mut knobs, mut events) = observed(Admission::AnyAttachment, None);
                    knobs.idle = idle;
                    knobs.recheck = Duration::from_millis(100);
                    served.serve_with(owner, knobs)?;
                    let mut client = attach_raw(&data, &scope, None).await?;
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::EnteredEmpty { reached: false },
                            ServeEvent::AttachmentAccepted { active: 1 },
                        ],
                    )
                    .await?;
                    let active_since = tokio::time::Instant::now();
                    // Existing owner events, rather than a fixture sleep, prove
                    // actual service availability across its empty-interval bound.
                    while active_since.elapsed() < idle {
                        ensure!(
                            events.recv().await == Some(ServeEvent::LockRechecked),
                            "active owner ended during its retention interval"
                        );
                    }
                    ensure!(
                        matches!(
                            rpc::request_attached(&mut client, &authority, ServiceCall::Revision)
                                .await?,
                            ServiceValue::Revision(_)
                        ),
                        "active retained owner could not answer"
                    );
                    drop(client);
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::AttachmentJoined { remaining: 0 },
                            ServeEvent::EnteredEmpty { reached: true },
                        ],
                    )
                    .await?;
                    // Reattach during the empty interval. A stale deadline must
                    // not retire this live attachment or its later empty period.
                    ensure!(
                        events.recv().await == Some(ServeEvent::LockRechecked),
                        "empty retained owner stopped accepting"
                    );
                    let first_empty = tokio::time::Instant::now();
                    let mut next = attach_raw(&data, &scope, None).await?;
                    expect_events(&mut events, &[ServeEvent::AttachmentAccepted { active: 1 }])
                        .await?;
                    while first_empty.elapsed() < idle {
                        ensure!(
                            events.recv().await == Some(ServeEvent::LockRechecked),
                            "reattached owner expired at its earlier deadline"
                        );
                    }
                    ensure!(
                        matches!(
                            rpc::request_attached(&mut next, &authority, ServiceCall::Revision)
                                .await?,
                            ServiceValue::Revision(_)
                        ),
                        "reattachment did not reuse the checked owner"
                    );
                    let final_empty = tokio::time::Instant::now();
                    drop(next);
                    expect_events(
                        &mut events,
                        &[
                            ServeEvent::AttachmentJoined { remaining: 0 },
                            ServeEvent::EnteredEmpty { reached: true },
                        ],
                    )
                    .await?;
                    served
                        .reap(
                            crate::server::close_budget() + idle,
                            "retained owner did not reap after its final interval",
                        )
                        .await?;
                    ensure!(
                        final_empty.elapsed() >= idle,
                        "idle deadline was not reset after reattachment"
                    );
                    ensure!(
                        EndpointRecord::read(&data, &scope)?.is_none(),
                        "retired owner retained discovery"
                    );
                    ensure!(
                        ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
                            .map(ServiceLock::release)
                            .transpose()?
                            .is_some(),
                        "reaped owner retained authority"
                    );
                    Ok(())
                },
                async |served| {
                    served
                        .retire(
                            &options,
                            Some(maintenance_deadline(&options)),
                            crate::server::close_budget(),
                            "retention fixture cleanup",
                        )
                        .await
                },
            )
            .await;
        root.release(outcome)
    }

    #[tokio::test]
    async fn idle_accept_deadlines_do_not_close_live_attachment() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner with a short lock recheck interval.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project)
                .await
                .map_err(|error| crate::test_support::fixture_startup_error(&options, error))?;
            let authority = owner.authority().clone();
            let (mut knobs, mut events) = observed(Admission::AnyAttachment, None);
            knobs.recheck = Duration::from_millis(100);
            let served = tokio::spawn(owner.serve_with(knobs));
            let mut client = attach_raw(&data, &scope, None).await?;
            expect_events(
                &mut events,
                &[
                    ServeEvent::EnteredEmpty { reached: false },
                    ServeEvent::AttachmentAccepted { active: 1 },
                ],
            )
            .await?;
            // Three rechecks after the accept: at least two accept deadlines
            // expired while this attachment stayed live and idle.
            for _ in 0..3 {
                ensure!(
                    events.recv().await == Some(ServeEvent::LockRechecked),
                    "a live attachment ended or the owner acted before three lock rechecks"
                );
            }
            ensure!(
                matches!(
                    rpc::request_attached(&mut client, &authority, ServiceCall::Revision).await?,
                    ServiceValue::Revision(_)
                ),
                "surviving attachment could not read after accept deadlines"
            );
            drop(client);
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("live attachment accept-timeout fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_owner_open_preserves_real_startup_log_when_annotated() -> Result<()> {
        let real_dolt = crate::test_support::warm_runtime_cache().await?;
        let root = tempfile::tempdir()?;
        let (project, _scope, _data, mut options) = owner_fixture(root.path())?;
        let failing_dolt = root.path().join("failing-dolt");
        let real_dolt = real_dolt.to_string_lossy().replace('\'', "'\\''");
        std::fs::write(
            &failing_dolt,
            format!(
                "#!/bin/sh\nif [ \"$1\" = version ]; then\n  exec '{real_dolt}' \"$@\"\nfi\nprintf '%s\\n' 'fixture-injected-dolt-startup-failure' >&2\nexit 1\n"
            ),
        )?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&failing_dolt, std::fs::Permissions::from_mode(0o700))?;
        }
        options.config.dolt_binary = Some(failing_dolt);

        let _gate = crate::spawn_gate::spawning().await;
        let raw_error = ServiceOwner::open(options.clone(), &project)
            .await
            .err()
            .context("fixture Dolt must fail before the owner is ready")?;
        let raw_rendered = format!("{raw_error:#}");
        ensure!(
            raw_rendered.contains("Dolt exited before readiness"),
            "fixture startup did not reach the Dolt readiness failure: {raw_rendered}"
        );
        ensure!(
            !raw_rendered.contains("fixture Dolt server log tail"),
            "the unannotated owner-open error unexpectedly included the fixture log"
        );
        let primary = raw_error.to_string();

        let annotated = crate::test_support::fixture_startup_error(&options, raw_error);
        let annotated_rendered = format!("{annotated:#}");
        ensure!(
            annotated_rendered.contains("fixture Dolt server log tail")
                && annotated_rendered.contains("fixture-injected-dolt-startup-failure"),
            "the fixture's bounded Dolt startup log was not retained: {annotated_rendered}"
        );
        ensure!(
            annotated
                .chain()
                .nth(1)
                .is_some_and(|cause| cause.to_string() == primary),
            "startup annotation replaced the original error: {annotated_rendered}"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn separate_cold_starters_share_one_owner_and_preserve_both_writes() -> Result<()> {
        use std::process::Stdio;
        use tokio::io::AsyncBufReadExt;
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh cold-started service owner, then the reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
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
            options.config.service_idle_timeout_secs = 0;
            options.supervisor = Some(executable.clone());
            let barrier = root.path().join("go");
            let ready_one = root.path().join("ready-one");
            let ready_two = root.path().join("ready-two");
            let _gate = crate::spawn_gate::spawning().await;
            let spawn = |ready: &Path| -> Result<tokio::process::Child> {
                let mut args = service_arguments(&options, &project, None);
                args[0] = "--internal-memory-service-client-fixture".into();
                args.push(barrier.as_os_str().to_owned());
                args.push(ready.as_os_str().to_owned());
                let child = tokio::process::Command::new(&executable)
                    .args(args)
                    .current_dir(&project)
                    .stdin(Stdio::piped())
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
            // Each starter reports its generation once attached and written,
            // then holds its attachment until its stdin closes: both are
            // attached at once, so both must reach the same owner.
            let (mut one, mut two) = (one, two);
            let mut one_lines = tokio::io::BufReader::new(
                one.stdout.take().context("first cold starter has no stdout")?,
            )
            .lines();
            let mut two_lines = tokio::io::BufReader::new(
                two.stdout.take().context("second cold starter has no stdout")?,
            )
            .lines();
            let (generation_one, generation_two) = tokio::join!(
                tokio::time::timeout(Duration::from_secs(60), one_lines.next_line()),
                tokio::time::timeout(Duration::from_secs(60), two_lines.next_line()),
            );
            let generation_one = generation_one
                .context("first cold starter did not report its generation")??
                .unwrap_or_default();
            let generation_two = generation_two
                .context("second cold starter did not report its generation")??
                .unwrap_or_default();
            // Release both only after reading both; an earlier failure return
            // kills them on drop.
            drop(one.stdin.take());
            drop(two.stdin.take());
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
            let exited = tokio::time::Instant::now();
            ensure!(
                !generation_one.is_empty() && generation_one == generation_two,
                "cold starters reached different service generations"
            );
            // Both starters have exited, so their owner retires at once.
            await_owner_release(&options).await?;
            let closed = exited.elapsed();
            let reopening = tokio::time::Instant::now();
            let mut client = attach_or_start(&options, &project, &executable).await?;
            let reopened = reopening.elapsed();
            ensure!(
                client.generation() != generation_one,
                "a retired cold service was still attached"
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
                "the successor lost a committed row"
            );
            drop(client);
            await_owner_release(&options).await?;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "cold-start fixture successor released its lock without retiring its endpoint"
            );
            eprintln!(
                "memory owner close after both starters exited: {} ms; reopen after the close: {} ms",
                closed.as_millis(),
                reopened.as_millis()
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("separate cold starter fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    /// A canonical project under `root`, its scope, private data directory
    /// and offline fixture options.
    pub(super) fn owner_fixture(
        root: &Path,
    ) -> Result<(PathBuf, String, PathBuf, crate::store::OpenOptions)> {
        let project = root.join("project");
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
        let data = root.join("private");
        let mut options = crate::store::OpenOptions::new(data.clone(), scope.clone());
        options.config.cache_dir = Some(crate::store::test_cache());
        options.config.offline = true;
        options.supervisor = Some(crate::store::test_supervisor()?);
        options.config.service_idle_timeout_secs = 0;
        Ok((project, scope, data, options))
    }

    /// An authenticated bare attachment to the published owner, presenting
    /// `starter_token` when given.
    pub(super) async fn attach_raw(
        data: &Path,
        scope: &str,
        starter_token: Option<uuid::Uuid>,
    ) -> Result<LocalStream> {
        let record = EndpointRecord::read(data, scope)?.context("missing service endpoint")?;
        let mut stream = connect_local(data, scope, &record.address, HANDSHAKE_TIMEOUT).await?;
        connect_handshake_presenting(&mut stream, &record.authority, starter_token).await?;
        Ok(stream)
    }

    async fn send_request(
        stream: &mut LocalStream,
        authority: &EndpointAuthority,
        call: ServiceCall,
    ) -> Result<()> {
        let request = rpc::ServiceRequest::new(&authority.service_generation, call);
        write_frame(stream, &request, 1024 * 1024, HANDSHAKE_TIMEOUT).await
    }

    /// Probe the owner lock, releasing it at once when free.
    pub(super) fn owner_lock_free(options: &crate::store::OpenOptions) -> Result<bool> {
        match ServiceLock::try_acquire(
            &options.data_dir,
            &options.project_scope,
            ServiceLockKind::Owner,
        )? {
            Some(lock) => lock.release().map(|()| true),
            None => Ok(false),
        }
    }

    /// Probe the store's lifecycle lease, which the Dolt reap releases,
    /// unlocking it at once when free.
    fn lifecycle_lease_free(options: &crate::store::OpenOptions) -> Result<bool> {
        let store = crate::store::project_directory(&options.data_dir, &options.project_scope)?;
        let directory = crate::files::directory(&store)?;
        #[cfg(unix)]
        let (locks, name) = (
            crate::files::directory(directory.path())?,
            OsString::from("lifecycle.lock"),
        );
        #[cfg(windows)]
        let (locks, name) = (
            crate::files::open_directory(
                &options.data_dir.join("memory/lifecycles"),
                Privacy::OwnerOnly,
                NameRetention::Pinned,
            )?,
            OsString::from(format!(
                "{}.lock",
                directory
                    .identity()
                    .to_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )),
        );
        let lock = locks.lock_file(&name)?;
        match lock.try_lock() {
            Ok(()) => {
                lock.unlock()?;
                Ok(true)
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(false),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    // T1
    #[tokio::test]
    async fn owner_retires_at_once_when_its_starter_detaches() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let token = uuid::Uuid::new_v4();
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            // A starter deadline far beyond the fixture backstop: only the
            // detach can end this owner in time.
            let (knobs, mut events) =
                observed(Admission::Starter(token), Some(Duration::from_secs(3600)));
            let served = tokio::spawn(owner.serve_with(knobs));
            let starter = attach_raw(&data, &scope, Some(token)).await?;
            expect_events(
                &mut events,
                &[
                    ServeEvent::EnteredEmpty { reached: false },
                    ServeEvent::AttachmentAccepted { active: 1 },
                ],
            )
            .await?;
            drop(starter);
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "retired owner kept its endpoint"
            );
            ensure!(
                lifecycle_lease_free(&options)?,
                "retired owner kept its lifecycle lease"
            );
            ensure!(owner_lock_free(&options)?, "retired owner kept its lock");
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("last-detach retirement fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // T2
    #[tokio::test]
    async fn owner_not_reached_by_its_starter_outlives_other_clients() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let token = uuid::Uuid::new_v4();
            let owner = ServiceOwner::open(options, &project).await?;
            let generation = owner.authority().service_generation.clone();
            let (knobs, mut events) =
                observed(Admission::Starter(token), Some(Duration::from_secs(3600)));
            let served = tokio::spawn(owner.serve_with(knobs));
            expect_events(&mut events, &[ServeEvent::EnteredEmpty { reached: false }]).await?;

            // A rejected handshake is an attachment only until it ends.
            let record = EndpointRecord::read(&data, &scope)?.context("missing endpoint")?;
            let mut wrong = record.authority.clone();
            wrong.connection_secret.push('x');
            let mut stranger =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            ensure!(
                connect_handshake(&mut stranger, &wrong).await.is_err(),
                "a wrong secret was admitted"
            );
            drop(stranger);
            let detached = [
                ServeEvent::AttachmentAccepted { active: 1 },
                ServeEvent::AttachmentJoined { remaining: 0 },
                ServeEvent::EnteredEmpty { reached: false },
            ];
            expect_events(&mut events, &detached).await?;

            // An authenticated client that is not the starter, such as an
            // inspection, attaches and detaches without retiring the owner.
            let inspection = attach_raw(&data, &scope, None).await?;
            drop(inspection);
            expect_events(&mut events, &detached).await?;
            ensure!(
                EndpointRecord::read(&data, &scope)?
                    .is_some_and(|record| record.authority.service_generation == generation),
                "a non-starter retired the fresh owner"
            );

            let starter = attach_raw(&data, &scope, Some(token)).await?;
            expect_events(&mut events, &[ServeEvent::AttachmentAccepted { active: 1 }]).await?;
            drop(starter);
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            ensure!(EndpointRecord::read(&data, &scope)?.is_none());
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("starter admission fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // T3
    #[tokio::test]
    async fn owner_whose_starter_never_attaches_exits_cleanly() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            // The starter's budget already elapsed at publication.
            let (knobs, mut events) = observed(
                Admission::Starter(uuid::Uuid::new_v4()),
                Some(Duration::ZERO),
            );
            let served = tokio::spawn(owner.serve_with(knobs));
            expect_events(&mut events, &[ServeEvent::EnteredEmpty { reached: false }]).await?;
            ensure!(
                next_event(&mut events).await.is_err(),
                "an owner past its starter deadline kept serving"
            );
            served
                .await?
                .context("a missing starter made the owner fail")?;
            ensure!(EndpointRecord::read(&data, &scope)?.is_none());
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("missing-starter fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    // T4
    #[tokio::test]
    async fn accepted_write_holds_the_owner_after_its_client_leaves() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, then its reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let authority = owner.authority().clone();
            let pause = Arc::new(rpc::RegisteredPause::default());
            owner.receipt_progress.pause_next(pause.clone());
            let (knobs, mut events) = observed(Admission::AnyAttachment, None);
            let served = tokio::spawn(owner.serve_with(knobs));
            let mut writer = attach_raw(&data, &scope, None).await?;
            let other = attach_raw(&data, &scope, None).await?;
            send_request(
                &mut writer,
                &authority,
                ServiceCall::AppendMessage {
                    namespace: "pending-work".into(),
                    message: kuru_core::Message::text("user", "accepted"),
                },
            )
            .await?;
            pause.entered.notified().await;
            // The writer's client vanishes with its request accepted.
            drop(writer);
            drop(other);
            expect_events(
                &mut events,
                &[
                    ServeEvent::EnteredEmpty { reached: false },
                    ServeEvent::AttachmentAccepted { active: 1 },
                    ServeEvent::AttachmentAccepted { active: 2 },
                    ServeEvent::AttachmentJoined { remaining: 1 },
                ],
            )
            .await?;
            pause.release.notify_one();
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            let ((), _gate) = crate::spawn_gate::excluding_spawns(gate, async {
                let successor = ServiceOwner::open(options, &project).await?;
                let window = successor
                    .inspection_store_for_test()
                    .history_window("pending-work", 4)
                    .await;
                let closed = successor.close().await;
                ensure!(
                    window?.total_rows == 1,
                    "the accepted write was not durable when the owner retired"
                );
                closed
            })
            .await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("pending-write fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    #[tokio::test]
    async fn accepted_read_holds_the_owner_until_it_is_answered() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, _options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(_options, &project).await?;
            let authority = owner.authority().clone();
            let pause = Arc::new(rpc::DispatchPause::default());
            let (mut knobs, mut events) = observed(Admission::AnyAttachment, None);
            knobs.dispatch_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            let mut reader = attach_raw(&data, &scope, None).await?;
            let other = attach_raw(&data, &scope, None).await?;
            send_request(&mut reader, &authority, ServiceCall::Revision).await?;
            pause.entered.notified().await;
            drop(reader);
            drop(other);
            expect_events(
                &mut events,
                &[
                    ServeEvent::EnteredEmpty { reached: false },
                    ServeEvent::AttachmentAccepted { active: 1 },
                    ServeEvent::AttachmentAccepted { active: 2 },
                    ServeEvent::AttachmentJoined { remaining: 1 },
                ],
            )
            .await?;
            pause.release.notify_one();
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("pending-read fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    // T5
    #[tokio::test]
    async fn dream_lease_holder_keeps_the_owner_and_its_candidate_survives() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, then its reopened successor.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let authority = owner.authority().clone();
            let (knobs, mut events) = observed(Admission::AnyAttachment, None);
            let served = tokio::spawn(owner.serve_with(knobs));
            let mut dreamer = attach_raw(&data, &scope, None).await?;
            ensure!(matches!(
                rpc::request_attached(&mut dreamer, &authority, ServiceCall::TryAcquireDreamLease)
                    .await?,
                ServiceValue::DreamLease { acquired: true }
            ));
            let mut candidate = attach_raw(&data, &scope, None).await?;
            let ServiceValue::CandidateStarted { handle, branch, .. } = rpc::request_attached(
                &mut candidate,
                &authority,
                ServiceCall::BeginCandidate {
                    label: "dream held".into(),
                },
            )
            .await?
            else {
                bail!("service did not start the dream candidate");
            };
            ensure!(matches!(
                rpc::request_attached(
                    &mut candidate,
                    &authority,
                    ServiceCall::View {
                        candidate: Some(handle),
                        operation: Box::new(rpc::ViewOperation::PutMany {
                            values: vec![("dream-private".into(), serde_json::json!(1))],
                        }),
                    },
                )
                .await?,
                ServiceValue::Unit
            ));
            let other = attach_raw(&data, &scope, None).await?;
            drop(candidate);
            drop(other);
            expect_events(
                &mut events,
                &[
                    ServeEvent::EnteredEmpty { reached: false },
                    ServeEvent::AttachmentAccepted { active: 1 },
                    ServeEvent::AttachmentAccepted { active: 2 },
                    ServeEvent::AttachmentAccepted { active: 3 },
                ],
            )
            .await?;
            let mut remaining = Vec::new();
            for _ in 0..2 {
                remaining.push(next_event(&mut events).await?);
            }
            ensure!(
                remaining
                    == [
                        ServeEvent::AttachmentJoined { remaining: 2 },
                        ServeEvent::AttachmentJoined { remaining: 1 },
                    ],
                "the lease holder's owner acted before it released: {remaining:?}"
            );
            drop(dreamer);
            expect_events(
                &mut events,
                &[
                    ServeEvent::AttachmentJoined { remaining: 0 },
                    ServeEvent::EnteredEmpty { reached: true },
                ],
            )
            .await?;
            served.await??;
            let ((), _gate) = crate::spawn_gate::excluding_spawns(gate, async {
                let successor = ServiceOwner::open(options, &project).await?;
                let inventory = successor
                    .inspection_store_for_test()
                    .candidate_inventory(None, 10)
                    .await;
                let closed = successor.close().await;
                ensure!(
                    inventory?
                        .candidates
                        .iter()
                        .any(|candidate| candidate.branch == branch),
                    "a lost candidate attachment took its ref with the owner"
                );
                closed
            })
            .await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("dream-lease fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    // T6
    #[tokio::test]
    async fn a_client_racing_shutdown_elects_a_successor_without_error() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh in-process owner; then, twice, a reopened in-process owner and
        // the reopened spawned successor a racing client elects.
        let deadline = crate::test_support::fixture_deadline(1, 4);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = crate::store::test_supervisor()?;
            let mut gate = crate::spawn_gate::spawning().await;

            // Connected but never accepted: the listener closes under it.
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let authority = owner.authority().clone();
            let pause = ClosePause::at(ClosePoint::BeforeListenerDrop);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;
            let record = EndpointRecord::read(&data, &scope)?.context("missing endpoint")?;
            let mut queued =
                connect_local(&data, &scope, &record.address, HANDSHAKE_TIMEOUT).await?;
            pause.release.notify_one();
            let error = connect_handshake(&mut queued, &authority)
                .await
                .err()
                .context("a closed listener completed a handshake")?;
            ensure!(
                is_peer_closed(&error),
                "an unaccepted connection to a retiring owner was not peer-closed: {error:#}"
            );
            served.await??;

            for point in [ClosePoint::AfterEndpointRetire, ClosePoint::AfterReap] {
                let (owner, next) = crate::spawn_gate::excluding_spawns(gate, async {
                    ServiceOwner::open(options.clone(), &project).await
                })
                .await?;
                gate = next;
                let generation = owner.authority().service_generation.clone();
                let pause = ClosePause::at(point);
                let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
                knobs.close_pause = Some(pause.clone());
                let served = tokio::spawn(owner.serve_with(knobs));
                drop(attach_raw(&data, &scope, None).await?);
                pause.entered.notified().await;
                // The endpoint is already retired, so one poll takes the start
                // lock and parks the client on the busy owner lock.
                let mut racing = Box::pin(attach_or_start(&options, &project, &executable));
                ensure!(
                    futures::poll!(racing.as_mut()).is_pending(),
                    "a client attached while the owner lock was still held at {point:?}"
                );
                ensure!(
                    ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?.is_none(),
                    "the racing client did not hold the start election at {point:?}"
                );
                pause.release.notify_one();
                let successor = racing
                    .await
                    .with_context(|| format!("racing client failed at {point:?}"))?;
                ensure!(
                    successor.generation() != generation,
                    "the racing client reached the retiring generation at {point:?}"
                );
                served.await??;
                drop(successor);
                await_owner_release(&options).await?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("shutdown race fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    // T6, the window between listener close and endpoint retirement: the
    // record still names the retiring generation, but nothing accepts.
    #[tokio::test]
    async fn a_client_meeting_a_record_without_a_listener_elects_a_successor() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh in-process owner and the reopened spawned
        // successor the racing client elects.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = crate::store::test_supervisor()?;
            let _gate = crate::spawn_gate::spawning().await;

            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let generation = owner.authority().service_generation.clone();
            let pause =
                ClosePause::at_each(&[ClosePoint::AfterListenerDrop, ClosePoint::AfterReap]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;

            let record = EndpointRecord::read(&data, &scope)?
                .context("the record was retired before the listener closed")?;
            ensure!(
                record.authority.service_generation == generation,
                "the published record named another generation"
            );
            let observed = try_attach_observed(&data, &scope, &project, None)
                .await
                .context("an electing client failed on a record without a listener")?;
            ensure!(
                matches!(observed, Err(AttachMiss::TransportUnavailable)),
                "a record without a listener was not a transport miss: {:?}",
                observed.as_ref().map(|_| ())
            );
            ensure!(
                request_idle_retirement(&options)
                    .await
                    .context("maintenance failed on a record without a listener")?
                    == RetirementReply::NoEndpoint,
                "maintenance read a record without a listener as an owner answer"
            );

            let mut racing = Box::pin(attach_or_start(&options, &project, &executable));
            // On Unix the refused connect returns at once, so one poll takes
            // the start lock and parks the client on the busy owner lock. A
            // Windows connect retries the absent pipe until its deadline, so
            // there the first poll is still inside that connect.
            #[cfg(unix)]
            {
                ensure!(
                    futures::poll!(racing.as_mut()).is_pending(),
                    "a client attached while the listener was closed"
                );
                ensure!(
                    ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?.is_none(),
                    "the racing client did not hold the start election"
                );
            }
            pause.release.notify_one();
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                attached = racing.as_mut() => {
                    let outcome = attached.map(|_| ());
                    bail!("the racing client finished before the owner reaped: {outcome:?}");
                }
            }
            ensure!(
                futures::poll!(racing.as_mut()).is_pending(),
                "a client finished while the owner lock was still held"
            );
            pause.release.notify_one();
            let successor = racing
                .await
                .context("the racing client failed after the owner released its lock")?;
            ensure!(
                successor.generation() != generation,
                "the racing client reached the retiring generation"
            );
            served.await??;
            drop(successor);
            await_owner_release(&options).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("listener-close race fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // T6m
    #[tokio::test]
    async fn maintenance_meeting_a_closing_owner_waits_for_its_lock() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, closed explicitly.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, _scope, _data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let mut owner = ServiceOwner::open(options.clone(), &project).await?;
            // The owner closes the maintenance connection without answering
            // it, as a retiring owner closes one it will never accept.
            let closed_peer = async {
                let stream = owner.accept(HANDSHAKE_TIMEOUT).await?;
                drop(stream);
                Ok::<(), anyhow::Error>(())
            };
            let (requested, closed) = tokio::join!(request_idle_retirement(&options), closed_peer);
            closed?;
            ensure!(
                requested.context("maintenance treated a closing owner as an error")?
                    == RetirementReply::PeerClosed,
                "maintenance read a closed connection as an owner answer"
            );
            owner.close().await?;
            drop(acquire_maintenance_permit(&options).await?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("maintenance race fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // Signature 2's routing on every host. A connect that a listener had
    // queued and then closed fails with a reset on Linux (CI job
    // 110146577721): that is a peer-closed miss for maintenance and for an
    // electing client, neither a fault nor "nothing is listening".
    #[test]
    fn a_connect_reset_by_a_retiring_owner_is_a_peer_closed_miss() {
        let reset = anyhow::Error::new(io::Error::from(io::ErrorKind::ConnectionReset));
        assert_eq!(connect_miss(&reset), Some(AttachMiss::PeerClosed));
        let reset = reset.context("connect to memory service for maintenance");
        assert!(is_peer_closed(&reset));
        assert!(!is_transport_unavailable(&reset));
        assert_eq!(connect_miss(&reset), Some(AttachMiss::PeerClosed));
        let refused = anyhow::Error::new(io::Error::from(io::ErrorKind::ConnectionRefused));
        assert_eq!(
            connect_miss(&refused),
            Some(AttachMiss::TransportUnavailable)
        );
        let denied = anyhow::Error::new(io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(connect_miss(&denied), None);
        assert_eq!(
            connect_miss(&anyhow::anyhow!("memory service connect deadline exceeded")),
            None
        );
    }

    /// A client stream whose peer disconnected it while its write was in
    /// flight, as Darwin reports one: `ENOTCONN`, not `EPIPE`.
    struct DisconnectedDuringWrite;

    impl AsyncRead for DisconnectedDuringWrite {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for DisconnectedDuringWrite {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            std::task::Poll::Ready(Err(io::Error::from(io::ErrorKind::NotConnected)))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    // CI runs 36910091434 and 37087605833 (macOS): a fixture's maintenance
    // request met an owner retiring on its own, and its hello write failed
    // "write memory service frame: Socket is not connected (os error 57)".
    // Darwin's `uipc_send` checks `SS_ISCONNECTED` before `SS_CANTSENDMORE`,
    // so a peer disconnect landing while `sosend` has the socket unlocked
    // reads as ENOTCONN rather than the EPIPE an earlier one gives. A socket
    // whose connect succeeded loses its connection only to the peer, so this
    // is the same peer-closed miss, for maintenance and an electing client.
    #[tokio::test]
    async fn a_handshake_write_disconnected_by_a_retiring_owner_is_a_peer_closed_miss() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            io::Error::from_raw_os_error(57).kind(),
            io::ErrorKind::NotConnected,
            "Darwin's ENOTCONN no longer maps to NotConnected"
        );
        let error = connect_handshake(&mut DisconnectedDuringWrite, &authority())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "write memory service frame");
        assert!(
            is_peer_closed(&error),
            "a disconnected handshake write was not a peer-closed miss: {error:#}"
        );
        assert!(!is_transport_unavailable(&error));
        assert_eq!(connect_miss(&error), Some(AttachMiss::PeerClosed));
        // Maintenance reads the same chain as the CI failure.
        let error = error.context("connect to memory service for maintenance");
        assert!(is_peer_closed(&error));
    }

    // A live owner that keeps closing connections while it holds its lock
    // looks, at each reset, like a retiring one. The lock decides: it is never
    // released, so maintenance fails at its deadline (the owner's close
    // budget, which a 1 s startup timeout does not shorten) and names the
    // requests that owner closed unanswered. The closed requests are real
    // socket replies, so this runs on the real clock for that budget rather
    // than auto-advancing paused time past an owner's reply.
    #[tokio::test]
    async fn maintenance_names_a_live_owner_that_keeps_closing_connections() -> Result<()> {
        // Held for the whole test: it holds a real owner flock and never
        // spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        let root = crate::test_support::tempdir()?;
        let (project, scope, data, mut options) = owner_fixture(root.path())?;
        options.config.startup_timeout_secs = 1;
        let owner = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire service owner lock")?;
        let (mut listener, address) = ServiceListener::bind(&data, &scope)?;
        let mut endpoint_authority = authority();
        endpoint_authority.project_path = project_path_bytes(&project);
        endpoint_authority.project_scope = scope.clone();
        let endpoint = EndpointRecord {
            authority: endpoint_authority,
            address,
        };
        endpoint.publish(&data, &owner)?;

        let trace = MaintenanceTrace::default();
        let closing = async {
            loop {
                if let Err(error) = listener.accept(HANDSHAKE_TIMEOUT).await {
                    return error;
                }
            }
        };
        let bound = maintenance_deadline(&options) + Duration::from_secs(2);
        let error = tokio::time::timeout(bound, async {
            tokio::select! {
                acquired = acquire_maintenance_permit_traced(&options, &trace) => {
                    acquired.map(drop).err().context("maintenance acquired a live owner's lock")
                }
                error = closing => Err(error).context("the closing owner stopped accepting"),
            }
        })
        .await
        .context("maintenance did not respect its owner deadline")??;
        let step = *trace
            .step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let message = format!("{error:#}");
        // The deadline falls either between requests or inside one.
        ensure!(
            message.contains("memory service owner is still active")
                || message.contains("memory maintenance owner-response deadline exceeded"),
            "maintenance did not report the live owner at its deadline: {message}"
        );
        ensure!(
            step.peer_closed > 0 && step.unanswered == 0,
            "the closed requests were not counted as peer-closed: {message}"
        );
        ensure!(
            message.contains(&format!(
                "requests the owner closed unanswered={}",
                step.peer_closed
            )),
            "the deadline error did not name the closed requests: {message}"
        );

        drop(listener);
        endpoint.retire(&data, &owner)?;
        Ok(())
    }

    // Signature 2 (CI job 110146577721): a maintenance connect that lands
    // after the retiring owner stopped accepting but before it dropped its
    // listener. Linux resets that queued connect; Darwin completes it and the
    // handshake meets the closed peer. Both are "no owner yet", and the owner
    // lock then orders maintenance after the reap.
    #[tokio::test]
    async fn maintenance_connect_queued_before_the_listener_dropped_waits_for_the_owner_lock()
    -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let generation = owner.authority().service_generation.clone();
            let pause = ClosePause::at_each(&[
                ClosePoint::BeforeListenerDrop,
                ClosePoint::AfterListenerDrop,
                ClosePoint::AfterReap,
            ]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;

            // Only Unix polls it before the listener drop.
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut requested = Box::pin(request_idle_retirement(&options));
            // On Unix one poll queues the connect on the listener that no
            // longer accepts and parks on its write readiness.
            #[cfg(unix)]
            ensure!(
                futures::poll!(requested.as_mut()).is_pending(),
                "the maintenance connect was not queued on the stopped listener"
            );
            pause.release.notify_one();
            pause.entered.notified().await;
            // The state a live owner that resets connections would also show.
            let record = EndpointRecord::read(&data, &scope)?
                .context("the record was retired before the listener closed")?;
            ensure!(
                record.authority.service_generation == generation,
                "the published record named another generation"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released with the listener"
            );
            let reply = requested
                .await
                .context("maintenance failed on a connect queued on a retiring owner")?;
            #[cfg(unix)]
            ensure!(
                reply == RetirementReply::PeerClosed,
                "a connect queued on a retiring owner was not peer-closed: {reply:?}"
            );
            ensure!(
                matches!(
                    reply,
                    RetirementReply::PeerClosed | RetirementReply::NoEndpoint
                ),
                "a retiring owner answered maintenance: {reply:?}"
            );

            let mut permit = Box::pin(acquire_maintenance_permit(&options));
            ensure!(
                futures::poll!(permit.as_mut()).is_pending(),
                "maintenance acquired while the retiring owner held its lock"
            );
            pause.release.notify_one();
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                acquired = permit.as_mut() => {
                    let outcome = acquired.map(drop);
                    bail!("maintenance finished before the owner reaped: {outcome:?}");
                }
            }
            ensure!(
                futures::poll!(permit.as_mut()).is_pending(),
                "maintenance acquired while the reaped owner still held its lock"
            );
            pause.release.notify_one();
            let permit = permit
                .await
                .context("maintenance failed after the owner released its lock")?;
            served.await??;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("queued maintenance connect fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // An owner closing on its own after its last client detached, held after
    // its Dolt reap and before its lock release for longer than
    // `startup_timeout_secs` but inside its own close budget
    // (`server::close_budget`): maintenance that meets that close waits for
    // the owner lock and is granted once the close ends. The acquisition's
    // waits are tokio timers and, at `AfterReap`, the endpoint record is
    // retired, so each request is a record read with no socket and nothing
    // live holds a timer: paused time carries the wait past the startup
    // timeout, and time resumes before the pause is released.
    #[tokio::test]
    async fn maintenance_behind_a_close_held_past_the_startup_timeout_acquires_within_the_close_budget()
    -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let held =
                Duration::from_secs(options.config.startup_timeout_secs) + Duration::from_secs(1);
            ensure!(
                held < crate::server::close_budget(),
                "the held close ({held:?}) is not inside the close budget"
            );
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let pause = ClosePause::at(ClosePoint::AfterReap);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;
            tokio::time::pause();
            let trace = MaintenanceTrace::default();
            let mut permit = Box::pin(acquire_maintenance_permit_traced(&options, &trace));
            let ended_early = tokio::select! {
                biased;
                acquired = permit.as_mut() => Some(acquired.map(drop)),
                () = tokio::time::sleep(held) => None,
            };
            tokio::time::resume();
            pause.release.notify_one();
            served.await??;
            if let Some(acquired) = ended_early {
                bail!(
                    "maintenance ended within {held:?} while the owner's close was held: {:?}",
                    acquired.map_err(|error| format!("{error:#}"))
                );
            }
            let permit = permit.await.with_context(|| {
                format!("maintenance failed once the held close ended; {trace}")
            })?;
            drop(permit);
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("held close maintenance fixture exceeded its {deadline:?} deadline")
        })?
    }

    // Signature 2, client half: an electing client's connect queued on the
    // stopped listener is a peer-closed miss, and that client then elects a
    // successor once the owner lock is released.
    #[tokio::test]
    async fn a_client_connect_queued_before_the_listener_dropped_is_a_peer_closed_miss()
    -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh in-process owner and the spawned
        // successor the racing client elects.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = crate::store::test_supervisor()?;
            let _gate = crate::spawn_gate::spawning().await;

            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let generation = owner.authority().service_generation.clone();
            let pause = ClosePause::at_each(&[
                ClosePoint::BeforeListenerDrop,
                ClosePoint::AfterListenerDrop,
                ClosePoint::AfterReap,
            ]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;

            // Only Unix polls it before the listener drop.
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut attaching = Box::pin(try_attach_observed(&data, &scope, &project, None));
            #[cfg(unix)]
            ensure!(
                futures::poll!(attaching.as_mut()).is_pending(),
                "the client connect was not queued on the stopped listener"
            );
            pause.release.notify_one();
            pause.entered.notified().await;
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released with the listener"
            );
            let observed = attaching
                .await
                .context("an electing client failed on a connect queued on a retiring owner")?;
            #[cfg(unix)]
            ensure!(
                matches!(observed, Err(AttachMiss::PeerClosed)),
                "a connect queued on a retiring owner was not a peer-closed miss: {:?}",
                observed.as_ref().map(|_| ())
            );
            ensure!(
                matches!(
                    observed,
                    Err(AttachMiss::PeerClosed | AttachMiss::TransportUnavailable)
                ),
                "a client attached to a retiring owner: {:?}",
                observed.as_ref().map(|_| ())
            );

            let mut racing = Box::pin(attach_or_start(&options, &project, &executable));
            #[cfg(unix)]
            {
                ensure!(
                    futures::poll!(racing.as_mut()).is_pending(),
                    "a client attached while the listener was closed"
                );
                ensure!(
                    ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?.is_none(),
                    "the racing client did not hold the start election"
                );
            }
            pause.release.notify_one();
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                attached = racing.as_mut() => {
                    let outcome = attached.map(|_| ());
                    bail!("the racing client finished before the owner reaped: {outcome:?}");
                }
            }
            ensure!(
                futures::poll!(racing.as_mut()).is_pending(),
                "a client finished while the owner lock was still held"
            );
            pause.release.notify_one();
            let successor = racing
                .await
                .context("the racing client failed after the owner released its lock")?;
            ensure!(
                successor.generation() != generation,
                "the racing client reached the retiring generation"
            );
            served.await??;
            drop(successor);
            await_owner_release(&options).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("queued client connect fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // T7
    #[tokio::test]
    async fn shutdown_retires_endpoint_then_reaps_then_releases_its_lock() -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner.
        let deadline = crate::test_support::fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let pause =
                ClosePause::at_each(&[ClosePoint::AfterEndpointRetire, ClosePoint::AfterReap]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "the endpoint outlived its retirement step"
            );
            ensure!(
                !lifecycle_lease_free(&options)?,
                "the lifecycle lease was released before the store closed"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released before the reap"
            );
            pause.release.notify_one();
            pause.entered.notified().await;
            ensure!(
                lifecycle_lease_free(&options)?,
                "the reap did not release the lifecycle lease"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released before the owner finished closing"
            );
            pause.release.notify_one();
            served.await??;
            ensure!(owner_lock_free(&options)?, "the closed owner kept its lock");
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("shutdown order fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    // T7i: a read-only inspection that arrives while the last client's owner
    // retires, as a fixture or CLI inspection does right after a command
    // exits. The service record is already gone but the store's own Dolt
    // endpoint stays published until the reap, so only the owner lock tells
    // that generation is still closing.
    #[tokio::test]
    async fn managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation()
    -> Result<()> {
        crate::test_support::warm_runtime_cache().await?;
        // Real lifecycles: one fresh service owner, and the inspection's local
        // reopen after that owner released its lock. The borrowing open
        // starts no Dolt.
        let deadline = crate::test_support::fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let mut inspection = options.clone();
            inspection.read_only = true;
            let dolt_endpoint =
                crate::store::project_directory(&data, &scope)?.join("endpoint.json");
            let gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let pause =
                ClosePause::at_each(&[ClosePoint::AfterEndpointRetire, ClosePoint::AfterReap]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;

            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "the service endpoint outlived its retirement step"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released before the reap"
            );
            ensure!(
                !lifecycle_lease_free(&options)?,
                "the lifecycle lease was released before the store closed"
            );
            ensure!(
                dolt_endpoint.exists(),
                "the store's Dolt endpoint was retired before the reap"
            );

            let mut managed = Box::pin(
                crate::MemoryStore::open_managed_observed(
                    inspection.clone(),
                    project.clone(),
                    crate::store::test_supervisor()?,
                )
                .1,
            );
            // Control, driven alongside the managed inspection: the direct
            // open consults no service authority, so it borrows the retiring
            // generation's still-published Dolt and reads through it. The
            // managed inspection, polled first, must still be waiting after
            // that complete borrowing open and read.
            let control = async {
                let borrowed = crate::MemoryStore::open(inspection.clone())
                    .await
                    .context("the direct read-only open did not borrow the retiring Dolt")?;
                let revision = borrowed.revision().await;
                Ok::<_, anyhow::Error>((borrowed, revision))
            };
            let (borrowed, revision) = tokio::select! {
                biased;
                opened = managed.as_mut() => {
                    let outcome = opened.map(|_| ());
                    bail!("the managed inspection opened while the retiring owner held its lock: {outcome:?}");
                }
                control = control => control?,
            };
            let revision = revision.context("the borrowed view did not read the retiring Dolt")?;
            pause.release.notify_one();
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                opened = managed.as_mut() => {
                    let outcome = opened.map(|_| ());
                    bail!("the managed inspection opened before the owner reaped: {outcome:?}");
                }
            }
            ensure!(
                lifecycle_lease_free(&options)?,
                "the reap did not release the lifecycle lease"
            );
            ensure!(
                !dolt_endpoint.exists(),
                "the reap left the store's Dolt endpoint published"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released before the owner finished closing"
            );
            ensure!(
                futures::poll!(managed.as_mut()).is_pending(),
                "a managed inspection opened while the reaped owner still held its lock"
            );
            // No Dolt runs now: the borrowed view outlived the generation it
            // read, while the managed inspection has not started its own.
            ensure!(
                borrowed.revision().await.is_err(),
                "a borrowed read-only view still read after its generation was reaped"
            );
            let _ = borrowed.close().await;

            // The owner lock release and the inspection's reacquisition of the
            // store's locks run with every other in-binary spawn excluded.
            let (memory, _gate) = crate::spawn_gate::excluding_spawns(gate, async {
                pause.release.notify_one();
                served.await??;
                managed.await
            })
            .await
            .context("the managed inspection failed after the owner released its lock")?;
            ensure!(
                format!("{memory:?}").contains(r#"backend: "local""#),
                "the inspection attached instead of opening its own generation: {memory:?}"
            );
            let read = memory.revision().await;
            let closed = memory.close().await;
            ensure!(
                read? == revision,
                "the inspection read another revision than the retired generation committed"
            );
            closed?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("retiring-owner inspection fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    // T12
    #[tokio::test]
    async fn inspection_skips_the_owner_probe_while_an_election_is_held() -> Result<()> {
        // Held for the whole test: it takes and releases real flocks and never
        // spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        let root = crate::test_support::tempdir()?;
        let (project, scope, data, mut options) = owner_fixture(root.path())?;
        options.read_only = true;
        options.config.startup_timeout_secs = 1;
        let start = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Start)?
            .context("fixture did not acquire the start lock")?;
        let error =
            tokio::time::timeout(Duration::from_secs(3), attach_existing(&options, &project))
                .await
                .context("inspection did not respect its startup deadline")?
                .err()
                .context("inspection concluded no owner while an election was held")?;
        ensure!(
            format!("{error:#}").contains("did not publish a readable endpoint"),
            "inspection did not report the held election: {error:#}"
        );
        start.release()?;
        ensure!(
            attach_existing(&options, &project).await?.is_none(),
            "inspection found an owner where none exists"
        );
        Ok(())
    }

    // T13
    #[test]
    fn released_lock_is_free_while_a_duplicate_descriptor_remains() -> Result<()> {
        let _gate = crate::spawn_gate::locking();
        let data = tempfile::tempdir()?;
        let scope = format!("project/{}", "f".repeat(64));
        let owner = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)?
            .context("fixture did not acquire the owner lock")?;
        // As a sibling's child holds it between fork and exec.
        let duplicate = owner.file().try_clone()?;
        owner.release()?;
        let reacquired = ServiceLock::try_acquire(data.path(), &scope, ServiceLockKind::Owner)?
            .context("a released lock stayed held by a duplicate descriptor")?;
        drop(duplicate);
        reacquired.release()
    }

    // T15
    #[test]
    fn starter_token_is_an_optional_argument_and_hello_field() -> Result<()> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let options = crate::store::OpenOptions::new(
            root.path().join("private"),
            format!("project/{}", "a".repeat(64)),
        );
        let token = uuid::Uuid::new_v4();
        let with = service_arguments(&options, &project, Some(token));
        let without = service_arguments(&options, &project, None);
        ensure!(with.len() == without.len());
        ensure!(parse_service_arguments(with[1..].to_vec())?.1.starter_token == Some(token));
        ensure!(
            parse_service_arguments(without[1..].to_vec())?
                .1
                .starter_token
                .is_none()
        );
        let mut extra = with[1..].to_vec();
        extra.push("x".into());
        ensure!(parse_service_arguments(extra).is_err());
        let mut invalid = without[1..].to_vec();
        invalid[8] = "not-a-token".into();
        ensure!(parse_service_arguments(invalid).is_err());
        // Legacy forms without idle or expected-instance arguments remain
        // accepted; only absent values gain the documented default.
        for length in [8, 9, 10] {
            let parsed = parse_service_arguments(without[1..1 + length].to_vec())?.1;
            ensure!(parsed.config.service_idle_timeout_secs == 30);
        }
        for seconds in [0, 30, 300] {
            let mut idle = without[1..].to_vec();
            idle[10] = seconds.to_string().into();
            ensure!(
                parse_service_arguments(idle)?
                    .1
                    .config
                    .service_idle_timeout_secs
                    == seconds
            );
        }
        for value in ["301", "-1", "1.5", "bad"] {
            let mut idle = without[1..].to_vec();
            idle[10] = value.into();
            ensure!(parse_service_arguments(idle).is_err());
        }

        let hello = authority().hello();
        let expected = format!(
            r#"{{"version":{{"major":{},"minor":{}}},"project_path":[47,112,114,105,118,97,116,101,47,112,114,111,106,101,99,116],"project_scope":"scope","history_scope":null,"store_instance":"store","service_generation":"generation","connection_secret":"test-secret","schema_version":4}}"#,
            PROTOCOL_MAJOR, PROTOCOL_MINOR
        );
        ensure!(
            serde_json::to_string(&hello)? == expected,
            "a hello without a starter token changed its wire form"
        );
        let mut presented = hello;
        presented.starter_token = Some(token.to_string());
        ensure!(!format!("{presented:?}").contains(&token.to_string()));
        let decoded: ClientHello = serde_json::from_slice(&serde_json::to_vec(&presented)?)?;
        ensure!(authority().verify(&decoded).is_ok());
        ensure!(decoded.starter_token == Some(token.to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn owner_learns_only_the_presented_starter_token() -> Result<()> {
        let expected = authority();
        let token = uuid::Uuid::new_v4();
        for presented in [None, Some(token)] {
            let (mut client, mut server) = duplex(1024);
            let server_authority = expected.clone();
            let accepted = tokio::spawn(async move {
                accept_handshake_presenting(&mut server, &server_authority).await
            });
            connect_handshake_presenting(&mut client, &expected, presented).await?;
            ensure!(accepted.await?? == Ok(presented.map(|token| token.to_string())));
        }
        Ok(())
    }

    #[tokio::test]
    async fn closing_an_attachment_drops_its_held_stream() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let data = tempfile::tempdir()?;
            let scope = format!("project/{}", "9".repeat(64));
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
                        .is_ok()
                );
                let _: ServiceRequest = read_frame(&mut stream, 1024, HANDSHAKE_TIMEOUT).await?;
                let _ = received.send(());
                // The client never reads a reply; its close ends this read.
                let mut rest = [0u8; 1];
                let closed = stream.read(&mut rest).await?;
                Ok::<usize, anyhow::Error>(closed)
            });
            let mut stream =
                connect_local(data.path(), &scope, &address, HANDSHAKE_TIMEOUT).await?;
            connect_handshake(&mut stream, &expected).await?;
            let mut attachment = ServiceAttachment {
                stream: Some(stream),
                held: None,
                retain_after_abandon: false,
                identity: AttachmentIdentity::new(),
                authority: expected,
                locator: None,
                last_fault: None,
                #[cfg(test)]
                reply_pause: None,
            };
            attachment.retain_after_abandon();
            let mut call = Box::pin(attachment.call(ServiceCall::Revision));
            tokio::select! {
                result = &mut call => bail!("client call unexpectedly completed: {result:?}"),
                ready = ready => ready.context("fixture server did not receive request")?,
            }
            drop(call);
            ensure!(
                attachment.stream.is_none() && attachment.holds_abandoned_stream(),
                "a cancelled call on a retaining attachment did not hold its stream"
            );
            ensure!(
                !server.is_finished(),
                "the held stream was closed before its attachment"
            );
            ensure!(attachment.call(ServiceCall::Revision).await.is_err());
            attachment.close();
            ensure!(!attachment.holds_abandoned_stream());
            ensure!(
                server.await?? == 0,
                "the owner saw more than the closed held stream"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("held stream fixture exceeded 10 seconds")??;
        Ok(())
    }
}
