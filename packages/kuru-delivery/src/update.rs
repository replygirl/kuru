//! Windows loaded-image replacement. The trusted, retained current-image helper
//! owns publication and its durable receipt; downloaded bytes are never run.

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, regular_file_info, seal_private,
    validate_component,
};
use kuru_platform::windows::{
    pipe::{self, Pipe, PrivateListener},
    process::{
        Lifetime, NativeSpawnSpec, Stdio, current_image, current_process_handle,
        duplicate_inherited_process_handle, wait_process_handle,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const STATE: &str = ".kuru-update";
const RECEIPT: &str = "receipt.json";
const LOCK: &str = "install.lock";
const STARTUP: Duration = Duration::from_secs(10);
// The authenticated helper verifies and durably copies full embedded images
// before acknowledging publication. That work has its own finite allowance;
// connection establishment and a frame already in flight keep their short limit.
const PUBLICATION: Duration = Duration::from_secs(120);
const CLEANUP: Duration = Duration::from_secs(10);
const JSON_LIMIT: usize = 64 * 1024;

// Ordinary callers supply a no-op. Maintainer fixtures observe completed
// filesystem boundaries without choosing outcomes or changing receipt bytes.
type Observer<'a> = dyn FnMut(&str, &Receipt) -> Result<()> + Send + 'a;

// A failed ACK can leave a correctly retained helper still doing work. Emit
// finite, static phase names before and after that work, without paths, hashes,
// credentials or a second control protocol. The existing bounded stderr owner
// drains these records; diagnostics never change publication or timeout policy.
struct HelperTrace(Instant);

impl HelperTrace {
    fn new() -> Self {
        Self(Instant::now())
    }

    fn at(&self, phase: &str) {
        let _ = writeln!(
            std::io::stderr().lock(),
            "trusted update helper: phase={phase} elapsed_ms={}",
            self.0.elapsed().as_millis()
        );
    }
}

#[derive(Debug)]
pub struct UpdateOutcome {
    pub installed: PathBuf,
    /// Publication was observed; the old running image may still need cleanup.
    pub cleanup_pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    OldMoved,
    Published,
    RolledBack,
    CleanupPending,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    identity: [u8; 24],
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    operation: String,
    parent: PathBuf,
    parent_identity: [u8; 24],
    installed: String,
    displaced: String,
    candidate: String,
    backup: String,
    original: Image,
    replacement: Image,
    rollback: Image,
    helper: PathBuf,
    helper_image: Image,
    #[serde(default)]
    restored: Option<Image>,
    phase: Phase,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    pipe: Option<OsString>,
    parent_handle: Option<usize>,
    parent_pid: Option<u32>,
    recovery: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    parent: PathBuf,
    parent_identity: [u8; 24],
    installed: String,
    original: Image,
    helper: PathBuf,
    helper_image: Image,
    candidate: PathBuf,
    candidate_image: Image,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgment {
    installed: PathBuf,
    image: Image,
}

fn open(path: &Path, private: bool) -> Result<Directory> {
    ensure!(path.is_absolute(), "update paths must be absolute");
    Ok(Directory::open(
        path,
        if private {
            Privacy::OwnerOnly
        } else {
            Privacy::Inherited
        },
        NameRetention::Movable,
    )?)
}

fn image(file: &mut File) -> Result<Image> {
    let metadata = regular_file_info(file)?;
    ensure!(
        metadata.len > 0 && metadata.len <= crate::archive::MAX_ARCHIVE_BYTES as u64,
        "update image exceeds bounds or is empty"
    );
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    (&mut *file)
        .take(crate::archive::MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len,
        "update image changed length"
    );
    Ok(Image {
        identity: metadata.identity.to_bytes(),
        sha256: crate::archive::digest(&bytes),
        bytes: metadata.len,
    })
}

fn verified(directory: &Directory, name: &str, expected: &Image) -> Result<File> {
    validate_component(OsStr::new(name))?;
    let mut file = directory.read(OsStr::new(name))?;
    let actual = image(&mut file)?;
    ensure!(
        actual.identity == expected.identity
            && actual.sha256 == expected.sha256
            && actual.bytes == expected.bytes,
        "update image identity or checksum changed: {name}"
    );
    directory.verify(OsStr::new(name), &file)?;
    Ok(file)
}

fn absent(directory: &Directory, name: &str) -> Result<bool> {
    match directory.read(OsStr::new(name)) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn write_new(
    directory: &Directory,
    name: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<(File, Image)> {
    let mut file = directory.create_new(OsStr::new(name))?;
    let result = (|| {
        file.write_all(bytes)?;
        if executable {
            kuru_platform::fs::make_executable(&file)?;
        }
        file.sync_all()?;
        image(&mut file)
    })();
    match result {
        Ok(record) => Ok((file, record)),
        Err(error) => {
            directory
                .remove_file(OsStr::new(name), file)
                .context("remove incomplete private update copy")?;
            Err(error)
        }
    }
}

fn copy_new(
    directory: &Directory,
    name: &str,
    source: &mut File,
    executable: bool,
) -> Result<(File, Image)> {
    let expected = image(source)?;
    source.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    source
        .take(crate::archive::MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == expected.bytes && crate::archive::digest(&bytes) == expected.sha256,
        "copy source changed"
    );
    write_new(directory, name, &bytes, executable)
}

fn save(directory: &Directory, receipt: &Receipt) -> Result<()> {
    let bytes = serde_json::to_vec(receipt)?;
    ensure!(bytes.len() <= JSON_LIMIT, "update receipt exceeds limit");
    let name = format!("receipt-{}.json", uuid::Uuid::new_v4());
    let mut file = directory.create_new(OsStr::new(&name))?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        if let Err(error) = directory.publish_file(
            directory,
            OsStr::new(&name),
            &file,
            OsStr::new(RECEIPT),
            Publication::ReplaceRegular,
        ) && directory.verify(OsStr::new(RECEIPT), &file).is_err()
        {
            return Err(error.into());
        }
        Ok(())
    })();
    if result.is_err() && directory.verify(OsStr::new(&name), &file).is_ok() {
        directory
            .remove_file(OsStr::new(&name), file)
            .context("remove unpublished receipt draft")?;
    }
    result
}

fn load(directory: &Directory) -> Result<Option<Receipt>> {
    let mut file = match directory.read(OsStr::new(RECEIPT)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(JSON_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= JSON_LIMIT, "update receipt exceeds limit");
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.schema_version == 1,
        "unsupported update receipt schema"
    );
    for name in [
        &receipt.installed,
        &receipt.displaced,
        &receipt.candidate,
        &receipt.backup,
    ] {
        validate_component(OsStr::new(name))?;
    }
    ensure!(
        receipt.installed == "kuru.exe"
            && receipt.displaced == format!(".kuru-old-{}.exe", receipt.operation)
            && receipt.candidate == format!("candidate-{}.exe", receipt.operation)
            && receipt.backup == format!("backup-{}.exe", receipt.operation),
        "invalid update transaction names"
    );
    uuid::Uuid::parse_str(&receipt.operation)?;
    directory.verify(OsStr::new(RECEIPT), &file)?;
    Ok(Some(receipt))
}

fn state(parent: &Directory) -> Result<Directory> {
    match open(&parent.path().join(STATE), true) {
        Ok(directory) => Ok(directory),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(parent.create_private_directory(OsStr::new(STATE))?)
        }
        Err(error) => Err(error),
    }
}

fn lock(directory: &Directory) -> Result<File> {
    let pinned = Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned)?;
    let file = pinned.lock_file(OsStr::new(LOCK))?;
    // Busy is a real unresolved operation, not a reason to overwrite its receipt.
    file.try_lock()
        .context("another update or recovery owns the installation")?;
    pinned.verify(OsStr::new(LOCK), &file)?;
    Ok(file)
}

/// All trusted native installation paths share the same Windows lease.
pub(crate) fn installation_guard(parent: &Directory) -> Result<File> {
    let directory = state(parent)?;
    let lease = lock(&directory)?;
    if let Some(mut receipt) = load(&directory)? {
        recover(&directory, &mut receipt)?;
        ensure!(
            matches!(receipt.phase, Phase::Complete | Phase::RolledBack),
            "prior update cleanup is pending; close old Kuru instances before replacement"
        );
        // This installation may replace the reconciled identity. A terminal
        // receipt must not later mistake that legitimate replacement for an
        // unknown occupant. Pending receipts are never retired here.
        directory.remove_file(OsStr::new(RECEIPT), directory.read(OsStr::new(RECEIPT))?)?;
    }
    Ok(lease)
}

fn checked_parent(receipt: &Receipt) -> Result<Directory> {
    let parent = open(&receipt.parent, false)?;
    ensure!(
        parent.identity().to_bytes() == receipt.parent_identity,
        "installation directory identity changed"
    );
    Ok(parent)
}

fn cleanup(directory: &Directory, receipt: &mut Receipt) -> Result<()> {
    cleanup_observed(directory, receipt, &mut |_, _| Ok(()))
}

fn cleanup_observed(
    directory: &Directory,
    receipt: &mut Receipt,
    observer: &mut Observer<'_>,
) -> Result<()> {
    let parent = checked_parent(receipt)?;
    drop(verified(&parent, &receipt.installed, &receipt.replacement)?);
    if !absent(&parent, &receipt.displaced)? {
        let old = verified(&parent, &receipt.displaced, &receipt.original)?;
        if parent
            .remove_file(OsStr::new(&receipt.displaced), old)
            .is_err()
        {
            receipt.phase = Phase::CleanupPending;
            save(directory, receipt)?;
            return Ok(());
        }
    }
    observer("old_removed_before_complete", receipt)?;
    if !absent(directory, &receipt.backup)? {
        let backup = verified(directory, &receipt.backup, &receipt.rollback)?;
        directory.remove_file(OsStr::new(&receipt.backup), backup)?;
    }
    receipt.phase = Phase::Complete;
    save(directory, receipt)
}

fn rolled_back(directory: &Directory, receipt: &mut Receipt) -> Result<()> {
    // These are only the exact private files recorded by this transaction. A
    // failed check retains the receipt; it never becomes a wildcard cleanup.
    for (name, expected) in [
        (&receipt.candidate, &receipt.replacement),
        (&receipt.backup, &receipt.rollback),
    ] {
        if !absent(directory, name)? {
            directory.remove_file(OsStr::new(name), verified(directory, name, expected)?)?;
        }
    }
    receipt.phase = Phase::RolledBack;
    save(directory, receipt)
}

/// Reconcile the namespace, including a receipt that lagged a successful move.
/// Unknown occupied names never grant rollback or deletion authority.
fn recover(directory: &Directory, receipt: &mut Receipt) -> Result<()> {
    let parent = checked_parent(receipt)?;
    let helper_parent = open(
        receipt.helper.parent().context("helper has no parent")?,
        true,
    )?;
    drop(verified(
        &helper_parent,
        receipt
            .helper
            .file_name()
            .and_then(OsStr::to_str)
            .context("invalid helper name")?,
        &receipt.helper_image,
    )?);
    if verified(&parent, &receipt.installed, &receipt.replacement).is_ok() {
        receipt.phase = Phase::Published;
        save(directory, receipt)?;
        return cleanup(directory, receipt);
    }
    if verified(&parent, &receipt.installed, &receipt.original).is_ok() {
        return rolled_back(directory, receipt);
    }
    if receipt
        .restored
        .as_ref()
        .is_some_and(|restored| verified(&parent, &receipt.installed, restored).is_ok())
    {
        return rolled_back(directory, receipt);
    }
    ensure!(
        absent(&parent, &receipt.installed)?,
        "unrecognized object occupies the installation; recovery retained"
    );
    if absent(&parent, &receipt.displaced)? {
        // The separately verified backup can recover bytes after a crash lost
        // the original object. Its new identity is explicit in the receipt.
        let mut backup = verified(directory, &receipt.backup, &receipt.rollback)?;
        let name = format!("restored-{}.exe", receipt.operation);
        let restored = if let Some(expected) = &receipt.restored {
            directory
                .read_write(OsStr::new(&name))
                .and_then(|file| {
                    directory.verify(OsStr::new(&name), &file)?;
                    Ok(file)
                })
                .map(|file| (file, expected.clone()))?
        } else {
            copy_new(directory, &name, &mut backup, true)?
        };
        let (file, record) = restored;
        drop(verified(directory, &name, &record)?);
        receipt.restored = Some(record);
        save(directory, receipt)?;
        if let Err(error) = parent.publish_file(
            directory,
            OsStr::new(&name),
            &file,
            OsStr::new(&receipt.installed),
            Publication::New,
        ) && parent
            .verify(OsStr::new(&receipt.installed), &file)
            .is_err()
        {
            return Err(error.into());
        }
        drop((file, backup));
        return rolled_back(directory, receipt);
    }
    let original = verified(&parent, &receipt.displaced, &receipt.original)?;
    if let Err(error) = parent.rename_file(
        &parent,
        OsStr::new(&receipt.displaced),
        &original,
        OsStr::new(&receipt.installed),
        Publication::New,
    ) && parent
        .verify(OsStr::new(&receipt.installed), &original)
        .is_err()
    {
        return Err(error.into());
    }
    drop(original);
    rolled_back(directory, receipt)
}

fn prepare(directory: &Directory, request: &Request, trace: &HelperTrace) -> Result<Receipt> {
    trace.at("prepare_parent_and_previous_receipt");
    let parent = open(&request.parent, false)?;
    ensure!(
        parent.identity().to_bytes() == request.parent_identity && request.installed == "kuru.exe",
        "installation identity changed"
    );
    if let Some(mut previous) = load(directory)? {
        recover(directory, &mut previous)?;
        ensure!(
            matches!(previous.phase, Phase::Complete | Phase::RolledBack),
            "prior update cleanup is pending; retry after old Kuru processes exit"
        );
    }
    trace.at("verify_original");
    let mut original = verified(&parent, &request.installed, &request.original)?;
    trace.at("verify_recorded_helper");
    let helper_parent = open(
        request.helper.parent().context("helper has no parent")?,
        true,
    )?;
    drop(verified(
        &helper_parent,
        request
            .helper
            .file_name()
            .and_then(OsStr::to_str)
            .context("invalid helper filename")?,
        &request.helper_image,
    )?);
    trace.at("verify_candidate_source");
    let candidate_parent = open(
        request
            .candidate
            .parent()
            .context("candidate has no parent")?,
        true,
    )?;
    let candidate_name = request
        .candidate
        .file_name()
        .and_then(OsStr::to_str)
        .context("candidate name is not UTF-8")?;
    let mut candidate = verified(&candidate_parent, candidate_name, &request.candidate_image)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let candidate_name = format!("candidate-{operation}.exe");
    let backup_name = format!("backup-{operation}.exe");
    trace.at("copy_candidate");
    let (replacement_file, replacement) =
        copy_new(directory, &candidate_name, &mut candidate, true)?;
    trace.at("copy_rollback");
    let (backup_file, rollback) = match copy_new(directory, &backup_name, &mut original, true) {
        Ok(value) => value,
        Err(error) => {
            directory
                .remove_file(OsStr::new(&candidate_name), replacement_file)
                .context("remove unpublished candidate after backup failure")?;
            return Err(error);
        }
    };
    let receipt = Receipt {
        schema_version: 1,
        displaced: format!(".kuru-old-{operation}.exe"),
        operation,
        parent: request.parent.clone(),
        parent_identity: request.parent_identity,
        installed: request.installed.clone(),
        candidate: candidate_name,
        backup: backup_name,
        original: request.original.clone(),
        replacement,
        rollback,
        helper: request.helper.clone(),
        helper_image: request.helper_image.clone(),
        restored: None,
        phase: Phase::Prepared,
    };
    trace.at("save_prepared_receipt");
    if let Err(error) = save(directory, &receipt) {
        // No rename of the installed image has happened yet. These retained
        // handles identify exactly the new files, including partial preparation.
        directory.remove_file(OsStr::new(&receipt.candidate), replacement_file)?;
        directory.remove_file(OsStr::new(&receipt.backup), backup_file)?;
        return Err(error);
    }
    drop((replacement_file, backup_file));
    trace.at("prepared_receipt_saved");
    Ok(receipt)
}

fn publish(
    directory: &Directory,
    receipt: &mut Receipt,
    observer: &mut Observer<'_>,
) -> Result<()> {
    move_original(directory, receipt, observer)?;
    publish_candidate(directory, receipt, observer)
}

fn move_original(
    directory: &Directory,
    receipt: &mut Receipt,
    observer: &mut Observer<'_>,
) -> Result<()> {
    let parent = checked_parent(receipt)?;
    let original = verified(&parent, &receipt.installed, &receipt.original)?;
    drop(verified(
        directory,
        &receipt.candidate,
        &receipt.replacement,
    )?);
    if let Err(error) = parent.rename_file(
        &parent,
        OsStr::new(&receipt.installed),
        &original,
        OsStr::new(&receipt.displaced),
        Publication::New,
    ) && parent
        .verify(OsStr::new(&receipt.displaced), &original)
        .is_err()
    {
        return Err(error.into());
    }
    drop(original);
    observer("old_moved_before_receipt", receipt)?;
    receipt.phase = Phase::OldMoved;
    save(directory, receipt)
}

fn publish_candidate(
    directory: &Directory,
    receipt: &mut Receipt,
    observer: &mut Observer<'_>,
) -> Result<()> {
    let parent = checked_parent(receipt)?;
    let replacement = directory.read_write(OsStr::new(&receipt.candidate))?;
    drop(verified(
        directory,
        &receipt.candidate,
        &receipt.replacement,
    )?);
    if let Err(error) = parent.publish_file(
        directory,
        OsStr::new(&receipt.candidate),
        &replacement,
        OsStr::new(&receipt.installed),
        Publication::New,
    ) && parent
        .verify(OsStr::new(&receipt.installed), &replacement)
        .is_err()
    {
        return Err(error.into());
    }
    drop(replacement);
    drop(verified(&parent, &receipt.installed, &receipt.replacement)?);
    observer("candidate_moved_before_receipt", receipt)?;
    receipt.phase = Phase::Published;
    save(directory, receipt)
}

async fn send<T: Serialize>(pipe: &mut Pipe, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= JSON_LIMIT,
        "update protocol message exceeds limit"
    );
    tokio::time::timeout(STARTUP, async {
        pipe.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
        pipe.write_all(&bytes).await?;
        pipe.flush().await
    })
    .await??;
    Ok(())
}

async fn receive<T: for<'de> Deserialize<'de>>(pipe: &mut Pipe) -> Result<T> {
    tokio::time::timeout(STARTUP, async {
        let first = pipe.read_u8().await?;
        receive_tail(pipe, first).await
    })
    .await?
}

async fn receive_tail<T: for<'de> Deserialize<'de>>(pipe: &mut Pipe, first: u8) -> Result<T> {
    let mut header = [first, 0, 0, 0];
    pipe.read_exact(&mut header[1..]).await?;
    let length = u32::from_le_bytes(header) as usize;
    ensure!(
        length <= JSON_LIMIT,
        "update protocol message exceeds limit"
    );
    let mut bytes = vec![0; length];
    pipe.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn receive_publication(
    pipe: &mut Pipe,
    publication: Duration,
    frame: Duration,
) -> Result<Acknowledgment> {
    let publication_deadline = tokio::time::Instant::now() + publication;
    let first = tokio::time::timeout_at(publication_deadline, pipe.read_u8())
        .await
        .context("publication acknowledgment did not begin within its budget")??;
    // A partial header cannot buy another publication allowance, and even a
    // late first byte cannot extend the total wait beyond the original budget.
    let frame_deadline = publication_deadline.min(tokio::time::Instant::now() + frame);
    tokio::time::timeout_at(frame_deadline, receive_tail(pipe, first))
        .await
        .context("publication acknowledgment frame did not complete within its budget")?
}

// The trusted helper has no provider or user-auth environment. Retain a bounded
// diagnostic prefix while continuously draining stderr, so an error can explain
// a failed handoff without making the helper depend on an available console.
async fn capture_helper_errors(
    mut pipe: Pipe,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<String> {
    let mut prefix = Vec::new();
    let mut buffer = [0_u8; 4096];
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut stop => break Ok(()),
            read = pipe.read(&mut buffer) => match read {
                Ok(0) => break Ok(()),
                Ok(count) => {
                    let keep = count.min(JSON_LIMIT.saturating_sub(prefix.len()));
                    prefix.extend_from_slice(&buffer[..keep]);
                }
                Err(error) => break Err(error),
            },
        }
    };
    let closed = pipe.close(CLEANUP).await;
    result.context("read trusted helper diagnostics")?;
    closed.context("close trusted helper diagnostics")?;
    Ok(String::from_utf8_lossy(&prefix).into_owned())
}

async fn helper_errors(
    stop: tokio::sync::oneshot::Sender<()>,
    mut task: tokio::task::JoinHandle<Result<String>>,
    exited: bool,
) -> String {
    // After exit the final native pipe data can still be buffered. Drain it to
    // EOF before stopping; otherwise stop could win select and hide the cause.
    let completed = if exited {
        tokio::time::timeout(CLEANUP, &mut task).await.ok()
    } else {
        None
    };
    let result = if let Some(result) = completed {
        result
    } else {
        let _ = stop.send(());
        task.await
    };
    match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => format!("diagnostic capture: {error:#}"),
        Err(error) => format!("diagnostic task: {error}"),
    }
}

pub async fn replace_running(
    base: &str,
    version: &str,
    helper_cache: &Path,
) -> Result<UpdateOutcome> {
    let target = crate::archive::host_target()?;
    let (bytes, support) = crate::archive::verified_release(base, version, target).await?;
    if let Some(files) = &support {
        let mut running =
            current_image().context("verify the loaded updater before support staging")?;
        let current = running.path().to_owned();
        let parent = current
            .parent()
            .context("loaded updater has no installation directory")?;
        ensure!(
            current.file_name() == Some(OsStr::new("kuru.exe")),
            "running updater must be installed as kuru.exe"
        );
        let _ = image(running.file_mut())?;
        crate::shell_support::install_versioned(files, parent, version, target)
            .context("shell support could not be staged; executable unchanged")?;
    }
    replace_bytes(&bytes, helper_cache).await
}

pub async fn replace_running_binary(
    candidate: &Path,
    helper_cache: &Path,
) -> Result<UpdateOutcome> {
    let bytes = candidate_bytes(candidate)?;
    replace_bytes(&bytes, helper_cache).await
}

fn candidate_bytes(candidate: &Path) -> Result<Vec<u8>> {
    let directory = open(
        candidate.parent().context("candidate has no parent")?,
        false,
    )?;
    let name = candidate.file_name().context("candidate has no filename")?;
    let mut file = directory.read(name)?;
    let expected = image(&mut file)?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::archive::MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == expected.bytes && crate::archive::digest(&bytes) == expected.sha256,
        "source build changed during verification"
    );
    directory.verify(name, &file)?;
    Ok(bytes)
}

async fn replace_bytes(bytes: &[u8], helper_cache: &Path) -> Result<UpdateOutcome> {
    let bytes = bytes.to_owned();
    let helper_cache = helper_cache.to_owned();
    // Accepted publication outlives cancellation of the UI/CLI waiter. Its
    // owned task retains the channel and scratch source until helper ownership
    // is acknowledged or its bounded request budget has ended.
    tokio::spawn(async move { replace_bytes_owned(&bytes, &helper_cache, |_| Ok(())).await })
        .await?
}

async fn replace_bytes_owned(
    bytes: &[u8],
    helper_cache: &Path,
    configure: impl FnOnce(&mut NativeSpawnSpec) -> Result<()> + Send,
) -> Result<UpdateOutcome> {
    let mut running = current_image().context("verify the actual loaded updater image")?;
    let current = running.path().to_owned();
    let parent = open(
        current.parent().context("current image has no parent")?,
        false,
    )?;
    ensure!(
        current.file_name() == Some(OsStr::new("kuru.exe")),
        "running image must be installed as kuru.exe"
    );
    let original = image(running.file_mut())?;
    let cache = Directory::ensure_private(helper_cache)?;
    let helper_name = format!("{}-{}.exe", crate::archive::host_target()?, original.sha256);
    let helper_image = if absent(&cache, &helper_name)? {
        let stage = crate::staging::Stage::create(&cache, ".helper-")?;
        let (file, record) = copy_new(stage.directory(), "helper.exe", running.file_mut(), true)?;
        seal_private(&file, true)?;
        cache.publish_file(
            stage.directory(),
            OsStr::new("helper.exe"),
            &file,
            OsStr::new(&helper_name),
            Publication::New,
        )?;
        drop(file);
        stage.finish()?;
        record
    } else {
        let mut held = cache.read(OsStr::new(&helper_name))?;
        let record = image(&mut held)?;
        ensure!(
            record.sha256 == original.sha256 && record.bytes == original.bytes,
            "cached trusted helper is corrupt; refusing execution"
        );
        record
    };
    let helper = cache.path().join(&helper_name);
    let helper_file = verified(&cache, &helper_name, &helper_image)?;
    // The trusted copy is complete. Release deny-delete sharing before the
    // helper moves the running original under the installation lease.
    drop(running);
    let stage = crate::staging::Stage::create(&parent, ".kuru-candidate-")?;
    let (candidate, candidate_image) = write_new(stage.directory(), "candidate.exe", bytes, true)?;
    drop(candidate);
    let listener = PrivateListener::bind()?;
    let parent_handle = current_process_handle()?;
    let start = Start {
        pipe: Some(listener.address().to_owned()),
        parent_handle: Some(parent_handle.as_raw_handle() as usize),
        parent_pid: Some(std::process::id()),
        recovery: None,
    };
    let mut spec = NativeSpawnSpec::new(helper.clone(), parent.path().to_owned());
    spec.lifetime = Lifetime::TrustedSupervisor;
    spec.stderr = Stdio::Pipe;
    // Keep instrumented child output in the runner's explicit coverage directory.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    spec.args = vec![
        "--internal-update-helper".into(),
        serde_json::to_string(&start)?.into(),
    ];
    spec.inherited.push(parent_handle);
    configure(&mut spec)?;
    let mut child = spec.spawn().await?;
    let errors = child.take_stderr().map(|pipe| {
        let (stop, receive) = tokio::sync::oneshot::channel();
        (stop, tokio::spawn(capture_helper_errors(pipe, receive)))
    });
    // Snapshot the retained peer identity before constructing the handoff
    // future. NativeChild owns asynchronous pipe state and is Send, not Sync;
    // the listener's owned accept future must not capture a shared child borrow.
    let accept = listener.accept(&child, STARTUP);
    let operation = async {
        let mut pipe = accept.await.context("accept trusted helper connection")?;
        send(
            &mut pipe,
            &Request {
                parent: parent.path().to_owned(),
                parent_identity: parent.identity().to_bytes(),
                installed: "kuru.exe".into(),
                original,
                helper,
                helper_image,
                candidate: stage.path().join("candidate.exe"),
                candidate_image,
            },
        )
        .await
        .context("send trusted helper update request")?;
        let acknowledgment = receive_publication(&mut pipe, PUBLICATION, STARTUP)
            .await
            .context("wait for verified update publication acknowledgment")?;
        ensure!(
            acknowledgment.installed == parent.path().join("kuru.exe"),
            "helper acknowledged an unexpected path"
        );
        ensure!(
            acknowledgment.image.bytes == bytes.len() as u64
                && acknowledgment.image.sha256 == crate::archive::digest(bytes),
            "helper acknowledged unexpected candidate bytes"
        );
        drop(verified(&parent, "kuru.exe", &acknowledgment.image)?);
        pipe.close(STARTUP).await?;
        Ok::<_, anyhow::Error>(UpdateOutcome {
            installed: acknowledgment.installed,
            cleanup_pending: true,
        })
    }
    .await;
    drop(helper_file);
    let stopped = if operation.is_err() {
        // The trusted helper exits after its bounded request/reconciliation
        // budget. Do not terminate it in the middle of the two-rename gap.
        Some(child.wait(STARTUP + CLEANUP).await)
    } else {
        None
    };
    let diagnostics = if let Some((stop, task)) = errors {
        helper_errors(stop, task, stopped.as_ref().is_some_and(Result::is_ok)).await
    } else {
        String::new()
    };
    // ACK proves the helper owns durable copies. On failure, only an observed
    // process exit proves it has stopped using our scratch source.
    let cleanup = if stopped.as_ref().is_some_and(Result::is_err) {
        let retained = stage.keep();
        Err(anyhow::anyhow!(
            "trusted helper exit was not observed; candidate stage retained at {}",
            retained.display()
        ))
    } else {
        stage.finish()
    };
    match operation {
        Err(error) => {
            let exit = match stopped.expect("failed handoff observes helper exit") {
                Ok(status) => format!("helper exit: {status}"),
                Err(error) => format!("helper exit unobserved: {error}"),
            };
            let cleanup = cleanup
                .err()
                .map_or_else(String::new, |error| format!("; {error:#}"));
            Err(error).context(format!(
                "update handoff failed; rerun the installer to reconcile any pending receipt; {exit}{cleanup}; helper stderr: {diagnostics}"
            ))
        }
        Ok(result) => {
            cleanup.context(
                "new executable is installed; candidate staging cleanup needs attention",
            )?;
            Ok(result)
        }
    }
}

/// Explicit internal mode used by the application and its native test fixture.
pub async fn run_helper(arguments: Vec<OsString>) -> Result<()> {
    run_helper_observed(arguments, &mut |_, _| Ok(())).await
}

async fn run_helper_observed(arguments: Vec<OsString>, observer: &mut Observer<'_>) -> Result<()> {
    let trace = HelperTrace::new();
    trace.at("started");
    ensure!(arguments.len() == 1, "invalid internal update invocation");
    let encoded = arguments[0]
        .to_str()
        .context("invalid helper request encoding")?;
    ensure!(encoded.len() <= JSON_LIMIT, "helper request exceeds limit");
    let start: Start = serde_json::from_str(encoded)?;
    if let Some(parent_path) = start.recovery {
        ensure!(
            start.pipe.is_none() && start.parent_handle.is_none() && start.parent_pid.is_none(),
            "invalid recovery invocation"
        );
        let parent = open(&parent_path, false)?;
        let directory = state(&parent)?;
        let _lease = lock(&directory)?;
        let mut receipt = load(&directory)?.context("no pending update receipt")?;
        let _running = checked_helper(&receipt.helper, &receipt.helper_image, &trace)?;
        recover(&directory, &mut receipt)?;
        ensure!(
            receipt.phase != Phase::CleanupPending,
            "old Kuru processes still hold the displaced image; recovery retained"
        );
        return Ok(());
    }
    let parent_handle = duplicate_inherited_process_handle(
        start.parent_handle.context("missing parent handle")?,
        start.parent_pid.context("missing parent identity")?,
    )?;
    trace.at("connect_parent");
    let mut pipe = pipe::connect(&start.pipe.context("missing helper channel")?, STARTUP).await?;
    trace.at("receive_request");
    let request: Request = receive(&mut pipe).await?;
    trace.at("open_installation_state");
    let parent = open(&request.parent, false)?;
    let directory = state(&parent)?;
    trace.at("acquire_installation_lock");
    let _lease = lock(&directory)?;
    trace.at("installation_lock_acquired");
    let _running = checked_helper(&request.helper, &request.helper_image, &trace)?;
    let mut receipt =
        prepare(&directory, &request, &trace).context("prepare durable update copies")?;
    observer("prepared", &receipt)?;
    trace.at("publish");
    let published = publish(&directory, &mut receipt, &mut |phase, receipt| {
        trace.at(phase);
        observer(phase, receipt)
    });
    if let Err(error) = published {
        trace.at("reconcile_publication_error");
        let reconciliation = recover(&directory, &mut receipt);
        if !matches!(
            receipt.phase,
            Phase::Published | Phase::CleanupPending | Phase::Complete
        ) || reconciliation.is_err()
        {
            return Err(error).context("publication failed; recovery receipt retained");
        }
    }
    observer("candidate_published", &receipt)?;
    trace.at("send_verified_publication_acknowledgment");
    send(
        &mut pipe,
        &Acknowledgment {
            installed: receipt.parent.join(&receipt.installed),
            image: receipt.replacement.clone(),
        },
    )
    .await
    .context("send acknowledgment after verified update publication")?;
    trace.at("verified_publication_acknowledgment_sent");
    pipe.close(STARTUP)
        .await
        .context("close acknowledged update channel")?;
    trace.at("wait_parent_exit");
    if wait_process_handle(&parent_handle, CLEANUP).await.is_err() {
        receipt.phase = Phase::CleanupPending;
        save(&directory, &receipt)?;
        trace.at("parent_still_alive_cleanup_pending");
        return Ok(());
    }
    trace.at("cleanup_after_parent_exit");
    cleanup_observed(&directory, &mut receipt, observer)?;
    trace.at("complete");
    Ok(())
}

fn checked_helper(
    path: &Path,
    expected: &Image,
    trace: &HelperTrace,
) -> Result<kuru_platform::windows::process::CurrentImage> {
    trace.at("acquire_loaded_helper_guard");
    let mut running = current_image().context("verify the actual loaded recovery helper")?;
    trace.at("hash_loaded_helper");
    let actual = image(running.file_mut())?;
    trace.at("verify_loaded_helper_record");
    let directory = open(path.parent().context("helper has no parent")?, true)?;
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("invalid helper filename")?;
    drop(verified(&directory, name, expected)?);
    ensure!(
        actual.identity == expected.identity
            && actual.sha256 == expected.sha256
            && actual.bytes == expected.bytes,
        "recovery must run the recorded trusted helper image"
    );
    trace.at("loaded_helper_verified");
    Ok(running)
}

/// Native fixture setup uses the actual preparation and first-move operations.
/// The application has no route to this maintainer-only entrypoint.
#[cfg(feature = "tooling")]
pub mod test_support {
    use super::*;

    /// Exercise actual native IPC framing with bounded fixture-specific clocks.
    pub async fn receive_publication_frame(
        pipe: &mut Pipe,
        publication: Duration,
        frame: Duration,
    ) -> Result<()> {
        receive_publication(pipe, publication, frame).await?;
        Ok(())
    }

    /// Exercise the production bounded prefix drain with actual native stderr.
    pub async fn capture_stderr(
        child: &mut kuru_platform::windows::process::NativeChild,
    ) -> Result<String> {
        let pipe = child.take_stderr().context("native stderr pipe")?;
        let (stop, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(capture_helper_errors(pipe, receive));
        let exit = child.wait(STARTUP + CLEANUP).await;
        let diagnostic = helper_errors(stop, task, exit.is_ok()).await;
        exit.context("native diagnostic fixture exit")?;
        Ok(diagnostic)
    }

    fn checked_checkpoint(value: &str) -> Result<()> {
        ensure!(
            matches!(
                value,
                "none"
                    | "prepared"
                    | "prepared_resume"
                    | "old_moved_before_receipt"
                    | "old_moved_before_receipt_resume"
                    | "candidate_moved_before_receipt"
                    | "candidate_published"
                    | "old_removed_before_complete"
            ),
            "unknown fixture update checkpoint"
        );
        Ok(())
    }

    /// The fixture alone selects a checkpoint and forwards its native stdio.
    /// Ordinary application invocation and serialized update receipts are unchanged.
    pub async fn replace_running_binary_observed(
        candidate: &Path,
        cache: &Path,
        checkpoint: &str,
    ) -> Result<UpdateOutcome> {
        checked_checkpoint(checkpoint)?;
        let bytes = candidate_bytes(candidate)?;
        let cache = cache.to_owned();
        let checkpoint = checkpoint.to_owned();
        tokio::spawn(async move {
            replace_bytes_owned(&bytes, &cache, move |spec| {
                use kuru_platform::windows::process::{StandardStream, inherited_stdio};
                spec.args
                    .extend(["--fixture-checkpoint".into(), checkpoint.into()]);
                spec.stdin = inherited_stdio(StandardStream::Input)?;
                spec.stdout = inherited_stdio(StandardStream::Output)?;
                spec.stderr = inherited_stdio(StandardStream::Error)?;
                // Only the fixture also forwards candidate-execution detection.
                if let Some(marker) = std::env::var_os("KURU_EXECUTION_MARKER") {
                    spec.environment
                        .push(("KURU_EXECUTION_MARKER".into(), marker));
                }
                Ok(())
            })
            .await
        })
        .await?
    }

    /// Called only by the compiled fixture after it removes its extra arguments.
    /// The production parser still receives exactly its original JSON argument.
    pub async fn run_helper_observed(arguments: Vec<OsString>, checkpoint: &str) -> Result<()> {
        checked_checkpoint(checkpoint)?;
        let (checkpoint, resume) = match checkpoint {
            "prepared_resume" => ("prepared", true),
            "old_moved_before_receipt_resume" => ("old_moved_before_receipt", true),
            other => (other, false),
        };
        let checkpoint = checkpoint.to_owned();
        let mut observer = move |reached: &str, receipt: &Receipt| {
            if reached == checkpoint {
                println!(
                    "{}",
                    serde_json::json!({"checkpoint":reached,"operation":receipt.operation,"phase":receipt.phase,"helper_pid":std::process::id()})
                );
                std::io::stdout().flush()?;
                // Interruption cases kill at this marker. Resume controls hold
                // either a real conflicting file handle or observed preparation
                // beyond startup before allowing actual publication to continue.
                let mut byte = [0];
                std::io::stdin().read_exact(&mut byte)?;
                ensure!(
                    resume && byte == [b'r'],
                    "fixture checkpoint was resumed instead of terminated"
                );
            }
            Ok(())
        };
        super::run_helper_observed(arguments, &mut observer).await
    }

    pub async fn hold_crash_gap(candidate: &Path, cache_path: &Path) -> Result<()> {
        let current = std::env::current_exe()?;
        let parent = open(
            current.parent().context("fixture image has no parent")?,
            false,
        )?;
        ensure!(
            current.file_name() == Some(OsStr::new("kuru.exe")),
            "crash fixture must be installed as kuru.exe"
        );
        let mut original_file = parent.read(OsStr::new("kuru.exe"))?;
        let original = image(&mut original_file)?;
        let cache = Directory::ensure_private(cache_path)?;
        let name = format!("{}-{}.exe", crate::archive::host_target()?, original.sha256);
        let (helper_file, helper_image) = copy_new(&cache, &name, &mut original_file, true)?;
        seal_private(&helper_file, true)?;
        helper_file.sync_all()?;
        let input_parent = open(
            candidate
                .parent()
                .context("fixture candidate has no parent")?,
            false,
        )?;
        let mut input = input_parent.read(
            candidate
                .file_name()
                .context("fixture candidate has no name")?,
        )?;
        let source = crate::staging::Stage::create(&parent, ".crash-input-")?;
        let (source_file, candidate_image) =
            copy_new(source.directory(), "candidate.exe", &mut input, true)?;
        drop(source_file);
        let directory = state(&parent)?;
        let _lease = lock(&directory)?;
        let mut receipt = prepare(
            &directory,
            &Request {
                parent: parent.path().to_owned(),
                parent_identity: parent.identity().to_bytes(),
                installed: "kuru.exe".into(),
                original,
                helper: cache.path().join(name),
                helper_image,
                candidate: source.path().join("candidate.exe"),
                candidate_image,
            },
            &HelperTrace::new(),
        )?;
        source.finish()?;
        drop((original_file, helper_file, input));
        move_original(&directory, &mut receipt, &mut |_, _| Ok(()))?;
        println!(
            "{}",
            serde_json::json!({"ready":"old_moved","receipt":directory.path().join(RECEIPT)})
        );
        std::io::stdout().flush()?;
        // The owning native test kills the fixture only after this acknowledgment.
        // No process timing or fabricated receipt can stand in for this phase.
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        parent: Directory,
        state: Directory,
        receipt: Receipt,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let parent = Directory::ensure_private(&root.path().join("installation")).unwrap();
            let state = state(&parent).unwrap();
            let inputs = Directory::ensure_private(&root.path().join("inputs")).unwrap();
            let (file, original) =
                write_new(&parent, "kuru.exe", b"old native image", true).unwrap();
            drop(file);
            let (file, candidate_image) =
                write_new(&inputs, "candidate.exe", b"new native image", true).unwrap();
            drop(file);
            let (file, helper_image) =
                write_new(&inputs, "helper.exe", b"old native image", true).unwrap();
            drop(file);
            let request = Request {
                parent: parent.path().to_owned(),
                parent_identity: parent.identity().to_bytes(),
                installed: "kuru.exe".into(),
                original,
                helper: inputs.path().join("helper.exe"),
                helper_image,
                candidate: inputs.path().join("candidate.exe"),
                candidate_image,
            };
            let receipt = prepare(&state, &request, &HelperTrace::new()).unwrap();
            Self {
                _root: root,
                parent,
                state,
                receipt,
            }
        }

        fn move_old(&self) {
            let file = verified(&self.parent, "kuru.exe", &self.receipt.original).unwrap();
            self.parent
                .rename_file(
                    &self.parent,
                    OsStr::new("kuru.exe"),
                    &file,
                    OsStr::new(&self.receipt.displaced),
                    Publication::New,
                )
                .unwrap();
        }
    }

    #[test]
    fn recovery_restores_original_identity_when_receipt_lags_first_move() {
        let mut fixture = Fixture::new();
        fixture.move_old();
        assert!(absent(&fixture.parent, "kuru.exe").unwrap());
        assert!(matches!(
            load(&fixture.state).unwrap().unwrap().phase,
            Phase::Prepared
        ));
        recover(&fixture.state, &mut fixture.receipt).unwrap();
        assert_eq!(fixture.receipt.phase, Phase::RolledBack);
        drop(verified(&fixture.parent, "kuru.exe", &fixture.receipt.original).unwrap());
        assert!(absent(&fixture.parent, &fixture.receipt.displaced).unwrap());
        assert!(absent(&fixture.state, &fixture.receipt.candidate).unwrap());
        assert!(absent(&fixture.state, &fixture.receipt.backup).unwrap());
    }

    #[test]
    fn recovery_preserves_published_candidate_when_receipt_lags_second_move() {
        let mut fixture = Fixture::new();
        fixture.move_old();
        let candidate = fixture
            .state
            .read_write(OsStr::new(&fixture.receipt.candidate))
            .unwrap();
        fixture
            .parent
            .publish_file(
                &fixture.state,
                OsStr::new(&fixture.receipt.candidate),
                &candidate,
                OsStr::new("kuru.exe"),
                Publication::New,
            )
            .unwrap();
        drop(candidate);
        recover(&fixture.state, &mut fixture.receipt).unwrap();
        assert_eq!(fixture.receipt.phase, Phase::Complete);
        drop(verified(&fixture.parent, "kuru.exe", &fixture.receipt.replacement).unwrap());
        assert!(absent(&fixture.parent, &fixture.receipt.displaced).unwrap());
    }

    #[test]
    fn ordinary_install_retires_only_reconciled_receipt_before_replacing_bytes() {
        for published in [false, true] {
            let mut fixture = Fixture::new();
            if published {
                publish(&fixture.state, &mut fixture.receipt, &mut |_, _| Ok(())).unwrap();
            }
            let lease = installation_guard(&fixture.parent).unwrap();
            assert!(load(&fixture.state).unwrap().is_none());
            assert!(absent(&fixture.state, &fixture.receipt.backup).unwrap());
            assert!(absent(&fixture.state, &fixture.receipt.candidate).unwrap());
            assert!(absent(&fixture.parent, &fixture.receipt.displaced).unwrap());
            // Replace through the ordinary checked path while holding the
            // same lease, then ensure its next acquisition has no stale ID.
            let (next, _) =
                write_new(&fixture.state, "ordinary.exe", b"next install", true).unwrap();
            fixture
                .parent
                .publish_file(
                    &fixture.state,
                    OsStr::new("ordinary.exe"),
                    &next,
                    OsStr::new("kuru.exe"),
                    Publication::ReplaceRegular,
                )
                .unwrap();
            drop((next, lease));
            drop(installation_guard(&fixture.parent).unwrap());
            assert_eq!(
                std::fs::read(fixture.parent.path().join("kuru.exe")).unwrap(),
                b"next install"
            );
        }
        let fixture = Fixture::new();
        let receipt = std::fs::read(fixture.state.path().join(RECEIPT)).unwrap();
        let lease = lock(&fixture.state).unwrap();
        assert!(installation_guard(&fixture.parent).is_err());
        assert_eq!(
            std::fs::read(fixture.state.path().join(RECEIPT)).unwrap(),
            receipt
        );
        drop(lease);
    }

    #[test]
    fn unknown_occupied_installation_is_never_overwritten_by_recovery() {
        let mut fixture = Fixture::new();
        fixture.move_old();
        let (foreign, identity) =
            write_new(&fixture.parent, "kuru.exe", b"unrecognized occupant", true).unwrap();
        drop(foreign);
        let before = std::fs::read(fixture.state.path().join(RECEIPT)).unwrap();
        assert!(recover(&fixture.state, &mut fixture.receipt).is_err());
        drop(verified(&fixture.parent, "kuru.exe", &identity).unwrap());
        drop(
            verified(
                &fixture.parent,
                &fixture.receipt.displaced,
                &fixture.receipt.original,
            )
            .unwrap(),
        );
        assert_eq!(
            std::fs::read(fixture.state.path().join(RECEIPT)).unwrap(),
            before
        );
    }

    #[test]
    fn lost_original_restores_verified_backup_bytes_with_an_explicit_new_identity() {
        let mut fixture = Fixture::new();
        fixture.move_old();
        let old = verified(
            &fixture.parent,
            &fixture.receipt.displaced,
            &fixture.receipt.original,
        )
        .unwrap();
        fixture
            .parent
            .remove_file(OsStr::new(&fixture.receipt.displaced), old)
            .unwrap();
        recover(&fixture.state, &mut fixture.receipt).unwrap();
        let restored = fixture.receipt.restored.as_ref().unwrap();
        assert_ne!(restored.identity, fixture.receipt.original.identity);
        assert_eq!(restored.sha256, fixture.receipt.original.sha256);
        assert_eq!(fixture.receipt.phase, Phase::RolledBack);
        drop(verified(&fixture.parent, "kuru.exe", restored).unwrap());
    }

    #[test]
    fn malformed_receipts_and_private_copy_failures_preserve_original_state() {
        let fixture = Fixture::new();
        let error = write_new(&fixture.state, "empty.exe", b"", true).unwrap_err();
        assert!(error.to_string().contains("empty"));
        assert!(absent(&fixture.state, "empty.exe").unwrap());
        let mut invalid = fixture.receipt.clone();
        invalid.displaced = "../outside.exe".into();
        save(&fixture.state, &invalid).unwrap();
        assert!(load(&fixture.state).is_err());
        drop(verified(&fixture.parent, "kuru.exe", &fixture.receipt.original).unwrap());
    }
}
