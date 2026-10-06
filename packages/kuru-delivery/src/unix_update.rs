//! Checked Unix update receipts. Recovery never opens memory or runs images.

use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, regular_file_info, validate_component,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const STATE: &str = ".kuru-update";
const RECEIPT: &str = "receipt.json";
const LOCK: &str = "install.lock";
const DRAFT: &str = "receipt.next";
const JSON_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("another update or recovery owns the installation")
    }
}
impl std::error::Error for Busy {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Prepared,
    Restoring,
    Published,
    RolledBack,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    identity: [u8; 24],
    sha256: String,
    bytes: u64,
}

impl Image {
    fn validate(&self) -> Result<()> {
        ensure!(self.bytes > 0, "update image is empty");
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "update image has an invalid checksum"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    version: Option<String>,
    target: String,
}

impl Meta {
    fn validate(&self) -> Result<()> {
        if let Some(version) = &self.version {
            crate::archive::checked_version(version)?;
        }
        crate::targets::find(&self.target)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    platform: String,
    operation: String,
    parent: PathBuf,
    parent_identity: [u8; 24],
    installed: String,
    candidate: String,
    backup: String,
    original: Image,
    original_meta: Meta,
    replacement: Option<Image>,
    replacement_meta: Meta,
    backup_image: Option<Image>,
    restored: Option<Image>,
    support_hashes: Option<[String; crate::shell_support::NAMES.len()]>,
    phase: Phase,
}

impl Receipt {
    fn validate(&self, parent: &Directory) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.platform == "unix",
            "update receipt needs a compatible newer Kuru; recovery state was retained"
        );
        let operation = Uuid::parse_str(&self.operation)?;
        ensure!(
            operation.to_string() == self.operation,
            "invalid update operation identity"
        );
        for name in [&self.installed, &self.candidate, &self.backup] {
            validate_component(OsStr::new(name))?;
        }
        ensure!(
            self.installed == "kuru"
                && self.candidate == format!("candidate-{operation}")
                && self.backup == format!("backup-{operation}"),
            "invalid update transaction names"
        );
        ensure!(
            self.parent == parent.path() && self.parent_identity == parent.identity().to_bytes(),
            "installation directory identity changed"
        );
        self.original.validate()?;
        self.original_meta.validate()?;
        self.replacement_meta.validate()?;
        if let Some(replacement) = &self.replacement {
            ensure!(
                replacement.bytes <= crate::archive::MAX_ARCHIVE_BYTES as u64,
                "update candidate exceeds existing archive bounds"
            );
        }
        for record in [&self.replacement, &self.backup_image, &self.restored]
            .into_iter()
            .flatten()
        {
            record.validate()?;
        }
        if matches!(
            self.phase,
            Phase::Prepared | Phase::Restoring | Phase::Published | Phase::Complete
        ) {
            ensure!(
                self.replacement.is_some() && self.backup_image.is_some(),
                "update receipt has incomplete prepared image evidence"
            );
        }
        if let Some(hashes) = &self.support_hashes {
            ensure!(
                self.replacement_meta.version.is_some(),
                "shell support needs an exact release version"
            );
            ensure!(
                hashes.iter().all(|hash| hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))),
                "invalid staged shell support checksum"
            );
        }
        if let Some(backup) = &self.backup_image {
            ensure!(
                backup.bytes == self.original.bytes && backup.sha256 == self.original.sha256,
                "update backup does not match the original image"
            );
        }
        if let Some(restored) = &self.restored {
            ensure!(
                restored.bytes == self.original.bytes && restored.sha256 == self.original.sha256,
                "restored image does not match the original image"
            );
        }
        Ok(())
    }
}

/// Hash from a retained checked file without allocating another image-sized buffer.
fn image(file: &mut File) -> Result<Image> {
    let before = regular_file_info(file)?;
    ensure!(
        before.links == 1 && before.len > 0,
        "update image must be a bounded single-link regular file"
    );
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 64 * 1024];
    let mut input = (&mut *file).take(
        before
            .len
            .checked_add(1)
            .context("update image length overflow")?,
    );
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .context("update image length overflow")?;
        ensure!(bytes <= before.len, "update image grew while reading");
        hash.update(&buffer[..read]);
    }
    let after = regular_file_info(file)?;
    ensure!(
        before.identity == after.identity && before.len == after.len && bytes == before.len,
        "update image changed while reading"
    );
    Ok(Image {
        identity: before.identity.to_bytes(),
        sha256: hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        bytes,
    })
}

fn verified(directory: &Directory, name: &str, expected: &Image) -> Result<File> {
    validate_component(OsStr::new(name))?;
    let mut file = directory.read(OsStr::new(name))?;
    ensure!(
        image(&mut file)? == *expected,
        "update image identity or checksum changed: {name}"
    );
    directory.verify(OsStr::new(name), &file)?;
    Ok(file)
}

fn optional_image(directory: &Directory, name: &str) -> Result<Option<Image>> {
    match directory.read(OsStr::new(name)) {
        Ok(mut file) => {
            let record = image(&mut file)?;
            directory.verify(OsStr::new(name), &file)?;
            Ok(Some(record))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn load_named(
    directory: &Directory,
    parent: &Directory,
    name: &str,
) -> Result<Option<(File, Receipt)>> {
    let mut file = match directory.read(OsStr::new(name)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(JSON_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= JSON_LIMIT, "update receipt exceeds limit");
    let receipt: Receipt = serde_json::from_slice(&bytes)
        .context("invalid update receipt; recovery state was retained")?;
    receipt.validate(parent)?;
    directory.verify(OsStr::new(name), &file)?;
    Ok(Some((file, receipt)))
}

fn compatible_draft(record: &Receipt, draft: &Receipt) -> Result<()> {
    ensure!(
        record.operation == draft.operation
            && record.original == draft.original
            && record.parent_identity == draft.parent_identity
            && record.original_meta == draft.original_meta
            && record.replacement_meta == draft.replacement_meta
            && record.support_hashes == draft.support_hashes
            && (record.replacement.is_none() || record.replacement == draft.replacement),
        "update receipt draft belongs to different evidence; recovery state was retained"
    );
    Ok(())
}

fn load(directory: &Directory, parent: &Directory) -> Result<Option<Receipt>> {
    let current = load_named(directory, parent, RECEIPT)?;
    let draft = load_named(directory, parent, DRAFT)?;
    match (current, draft) {
        (Some((_, record)), Some((_, draft))) => {
            compatible_draft(&record, &draft)?;
            Ok(Some(record))
        }
        (Some((_, record)), None) | (None, Some((_, record))) => Ok(Some(record)),
        (None, None) => Ok(None),
    }
}

fn save(directory: &Directory, parent: &Directory, receipt: &Receipt) -> Result<()> {
    receipt.validate(parent)?;
    if let Some((file, previous)) = load_named(directory, parent, DRAFT)? {
        compatible_draft(&previous, receipt)?;
        directory.remove_file(OsStr::new(DRAFT), file)?;
    }
    let bytes = serde_json::to_vec(receipt)?;
    ensure!(bytes.len() <= JSON_LIMIT, "update receipt exceeds limit");
    let mut file = directory.create_new(OsStr::new(DRAFT))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    // A crash leaves one fixed attributable draft; incomplete or mismatched
    // drafts refuse instead of being mistaken for disposable foreign files.
    directory.publish_file(
        directory,
        OsStr::new(DRAFT),
        &file,
        OsStr::new(RECEIPT),
        Publication::ReplaceRegular,
    )?;
    Ok(())
}

struct State {
    parent: Directory,
    private: Directory,
    ordinary: Directory,
    _lock: File,
}

impl State {
    fn open(parent: &Path, create: bool) -> Result<Option<Self>> {
        let parent = Directory::open(parent, Privacy::Inherited, NameRetention::Movable)?;
        let private = match Directory::open(
            &parent.path().join(STATE),
            Privacy::OwnerOnly,
            NameRetention::Movable,
        ) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                parent.create_private_directory(OsStr::new(STATE))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if private
            .read(OsStr::new(RECEIPT))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            && private
                .read(OsStr::new(DRAFT))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            // No Preparing receipt means no payload can be attributed to an
            // interrupted update. Refuse before even creating the lock.
            private.require_known_entries(&[OsStr::new(LOCK)])?;
            match private.read(OsStr::new(LOCK)) {
                Ok(file) => {
                    ensure!(
                        regular_file_info(&file)?.len == 0,
                        "unrecognized update lock bytes"
                    );
                    private.require_owned_replacement(OsStr::new(LOCK), &file)?;
                    private.verify(OsStr::new(LOCK), &file)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            private.revalidate()?;
            if !create {
                return Ok(None);
            }
        }
        // Native identity contains the volume; publish_file rechecks it too.
        ensure!(
            private.identity().to_bytes()[..8] == parent.identity().to_bytes()[..8],
            "update state must stay on the installation filesystem"
        );
        let ordinary = Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable)?;
        ensure!(
            ordinary.identity() == private.identity(),
            "update state identity changed"
        );
        let lock = private.lock_file(OsStr::new(LOCK))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(Busy.into()),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        private.verify(OsStr::new(LOCK), &lock)?;
        Ok(Some(Self {
            parent,
            private,
            ordinary,
            _lock: lock,
        }))
    }
}

/// Inputs have been selected by the caller; neither variant is ever executed.
pub enum Candidate {
    Bytes(Vec<u8>),
    BuildInput(PathBuf),
}

pub struct Request {
    pub installation: crate::ownership::Installation,
    pub candidate: Candidate,
    pub version: Option<String>,
    pub target: String,
    pub support: Option<crate::shell_support::Files>,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub installed: PathBuf,
    pub published: bool,
    /// The executable is settled even if stable man publication failed.
    pub support_incomplete: bool,
}

fn candidate_bytes(candidate: Candidate) -> Result<Vec<u8>> {
    let bytes = match candidate {
        Candidate::Bytes(bytes) => bytes,
        Candidate::BuildInput(path) => {
            ensure!(path.is_absolute(), "build input path must be absolute");
            let parent = Directory::open(
                path.parent().context("build input has no parent")?,
                Privacy::Inherited,
                NameRetention::Movable,
            )?;
            let name = path.file_name().context("build input has no name")?;
            let mut input = crate::archive::open_build_input(&parent, name)?;
            let before = regular_file_info(&input)?;
            let mut bytes = Vec::new();
            (&mut input)
                .take(crate::archive::MAX_ARCHIVE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            crate::archive::verify_build_snapshot(&parent, name, &mut input, before, &bytes)?;
            bytes
        }
    };
    ensure!(
        !bytes.is_empty() && bytes.len() <= crate::archive::MAX_ARCHIVE_BYTES,
        "update candidate exceeds bounds or is empty"
    );
    Ok(bytes)
}

fn write_image(directory: &Directory, name: &str, bytes: &[u8], private: bool) -> Result<Image> {
    let mut file = directory.create_new(OsStr::new(name))?;
    file.write_all(bytes)?;
    if private {
        kuru_platform::fs::seal_private(&file, true)?;
    } else {
        kuru_platform::fs::make_executable(&file)?;
    }
    file.sync_all()?;
    let record = image(&mut file)?;
    ensure!(
        record.bytes == bytes.len() as u64 && record.sha256 == crate::archive::digest(bytes),
        "update copy bytes changed"
    );
    directory.verify(OsStr::new(name), &file)?;
    Ok(record)
}

fn copy_image(
    directory: &Directory,
    name: &str,
    source: &mut File,
    expected: &Image,
    private: bool,
) -> Result<Image> {
    ensure!(image(source)? == *expected, "held original image changed");
    source.seek(SeekFrom::Start(0))?;
    let mut output = directory.create_new(OsStr::new(name))?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0; 64 * 1024];
    let mut input = (&mut *source).take(
        expected
            .bytes
            .checked_add(1)
            .context("update copy length overflow")?,
    );
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("update copy length overflow")?;
        ensure!(total <= expected.bytes, "held original grew during copy");
        hash.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    let copied_hash: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        total == expected.bytes && copied_hash == expected.sha256 && image(source)? == *expected,
        "held original bytes changed during copy"
    );
    if private {
        kuru_platform::fs::seal_private(&output, true)?;
    } else {
        kuru_platform::fs::make_executable(&output)?;
    }
    output.sync_all()?;
    let copied = image(&mut output)?;
    ensure!(
        copied.bytes == expected.bytes && copied.sha256 == expected.sha256,
        "update copy bytes changed"
    );
    directory.verify(OsStr::new(name), &output)?;
    Ok(copied)
}

fn remove_verified(directory: &Directory, name: &str, expected: &Image) -> Result<()> {
    match directory.read(OsStr::new(name)) {
        Ok(mut file) => {
            ensure!(
                image(&mut file)? == *expected,
                "update cleanup image changed: {name}"
            );
            directory.remove_file(OsStr::new(name), file)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn check_cleanup(
    directory: &Directory,
    name: &str,
    expected: Option<&Image>,
    maximum: u64,
) -> Result<()> {
    match directory.read(OsStr::new(name)) {
        Ok(mut file) => {
            if let Some(expected) = expected {
                ensure!(
                    image(&mut file)? == *expected,
                    "update cleanup image changed: {name}"
                );
            } else {
                ensure!(
                    regular_file_info(&file)?.len <= maximum,
                    "incomplete update copy violates captured bounds"
                );
                directory.require_owned_replacement(OsStr::new(name), &file)?;
            }
            directory.verify(OsStr::new(name), &file)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn remove_preparing(directory: &Directory, name: &str, maximum: u64) -> Result<()> {
    match directory.read(OsStr::new(name)) {
        Ok(file) => {
            let record = regular_file_info(&file)?;
            ensure!(
                record.links == 1 && record.len <= maximum,
                "incomplete update copy violates captured bounds"
            );
            directory.require_owned_replacement(OsStr::new(name), &file)?;
            directory.remove_file(OsStr::new(name), file)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn retire(state: &State, receipt: &Receipt) -> Result<()> {
    for name in [DRAFT, RECEIPT] {
        if let Some((file, record)) = load_named(&state.private, &state.parent, name)? {
            compatible_draft(receipt, &record)?;
            state.private.remove_file(OsStr::new(name), file)?;
        }
    }
    Ok(())
}

fn stable_support(state: &State, receipt: &Receipt) -> Result<()> {
    let Some(hashes) = &receipt.support_hashes else {
        return Ok(());
    };
    let version = receipt
        .replacement_meta
        .version
        .as_deref()
        .context("shell support needs an exact release version")?;
    let root = state
        .parent
        .path()
        .join("share/kuru")
        .join(version)
        .join(&receipt.replacement_meta.target);
    let files = crate::shell_support::read_generated(&root)?;
    ensure!(
        files
            .0
            .iter()
            .zip(hashes)
            .all(|(bytes, expected)| crate::archive::digest(bytes) == *expected),
        "staged shell support changed; executable is installed"
    );
    crate::shell_support::publish_stable_man(&files, state.parent.path())?;
    Ok(())
}

fn settle(state: &State, receipt: &mut Receipt, published: bool) -> Result<Outcome> {
    settle_observed(state, receipt, published, &mut |_| Ok(()))
}

fn settle_observed(
    state: &State,
    receipt: &mut Receipt,
    published: bool,
    observer: &mut dyn FnMut(Checkpoint) -> Result<()>,
) -> Result<Outcome> {
    let installed = optional_image(&state.parent, &receipt.installed)?;
    let expected = if published {
        receipt
            .replacement
            .as_ref()
            .context("missing replacement evidence")?
    } else if installed.as_ref() == Some(&receipt.original) {
        &receipt.original
    } else {
        receipt
            .restored
            .as_ref()
            .context("missing restored installation evidence")?
    };
    ensure!(
        installed.as_ref() == Some(expected),
        "installed executable changed before settlement"
    );
    let restored_name = format!("restored-{}", receipt.operation);
    let partial_restore = receipt.phase == Phase::Restoring && receipt.restored.is_none();
    if receipt.restored.is_some() || partial_restore {
        check_cleanup(
            &state.ordinary,
            &restored_name,
            receipt.restored.as_ref(),
            receipt.original.bytes,
        )?;
    } else {
        ensure!(
            optional_image(&state.ordinary, &restored_name)?.is_none(),
            "unaccounted restored image; recovery state was retained"
        );
    }
    // Refuse already-observed unknown payloads before changing the receipt.
    // Removal checks the exact handles/bytes again at its own effect boundary.
    check_cleanup(
        &state.ordinary,
        &receipt.candidate,
        receipt.replacement.as_ref(),
        crate::archive::MAX_ARCHIVE_BYTES as u64,
    )?;
    check_cleanup(
        &state.private,
        &receipt.backup,
        receipt.backup_image.as_ref(),
        receipt.original.bytes,
    )?;
    // Identity establishes which effect occurred, not directory durability.
    // Retry the actual retained-directory sync before claiming settlement.
    state.private.sync()?;
    state.parent.sync()?;
    // Keep durable Restoring intent until its unrecorded partial image is gone.
    if let Some(restored) = &receipt.restored {
        remove_verified(&state.ordinary, &restored_name, restored)?;
    } else if partial_restore {
        remove_preparing(&state.ordinary, &restored_name, receipt.original.bytes)?;
    }
    if published {
        receipt.phase = Phase::Published;
        save(&state.private, &state.parent, receipt)?;
        observer(Checkpoint::Published)?;
    }
    let support_incomplete = published && stable_support(state, receipt).is_err();
    if let Some(replacement) = &receipt.replacement {
        remove_verified(&state.ordinary, &receipt.candidate, replacement)?;
    } else {
        remove_preparing(
            &state.ordinary,
            &receipt.candidate,
            crate::archive::MAX_ARCHIVE_BYTES as u64,
        )?;
    }
    if let Some(backup) = &receipt.backup_image {
        remove_verified(&state.private, &receipt.backup, backup)?;
    } else {
        remove_preparing(&state.private, &receipt.backup, receipt.original.bytes)?;
    }
    observer(Checkpoint::BackupRemoved)?;
    receipt.phase = if published {
        Phase::Complete
    } else {
        Phase::RolledBack
    };
    save(&state.private, &state.parent, receipt)?;
    retire(state, receipt)?;
    Ok(Outcome {
        installed: state.parent.path().join(&receipt.installed),
        published,
        support_incomplete,
    })
}

fn recover_locked(state: &State, receipt: &mut Receipt) -> Result<Outcome> {
    let installed = optional_image(&state.parent, &receipt.installed)?;
    if installed.as_ref() == receipt.replacement.as_ref() && installed.is_some() {
        return settle(state, receipt, true);
    }
    if installed.as_ref() == Some(&receipt.original)
        || (installed.is_some() && installed.as_ref() == receipt.restored.as_ref())
    {
        return settle(state, receipt, false);
    }
    if installed.is_some() {
        anyhow::bail!(
            "installed executable differs from update evidence; recovery state was retained"
        );
    }
    ensure!(
        matches!(receipt.phase, Phase::Prepared | Phase::Restoring),
        "installed executable is absent after possible update publication; reinstall an explicitly selected release; recovery state was retained"
    );
    let replacement = receipt
        .replacement
        .as_ref()
        .context("missing replacement evidence")?;
    // Candidate presence proves the atomic move did not consume it.
    drop(verified(&state.ordinary, &receipt.candidate, replacement)
        .context("installed executable is absent after possible update publication; recovery state was retained")?);
    let backup = receipt
        .backup_image
        .as_ref()
        .context("missing backup evidence")?;
    let mut original = verified(&state.private, &receipt.backup, backup)?;
    let restored_name = format!("restored-{}", receipt.operation);
    let restored = if let Some(expected) = &receipt.restored {
        drop(verified(&state.ordinary, &restored_name, expected)?);
        expected.clone()
    } else {
        if receipt.phase == Phase::Prepared {
            ensure!(
                optional_image(&state.ordinary, &restored_name)?.is_none(),
                "unaccounted restored image; recovery state was retained"
            );
            receipt.phase = Phase::Restoring;
            save(&state.private, &state.parent, receipt)?;
        } else {
            // The exact restoration intent was durable before this partial copy.
            remove_preparing(&state.ordinary, &restored_name, receipt.original.bytes)?;
        }
        let restored = copy_image(
            &state.ordinary,
            &restored_name,
            &mut original,
            backup,
            false,
        )?;
        receipt.restored = Some(restored.clone());
        save(&state.private, &state.parent, receipt)?;
        restored
    };
    let restored_file = verified(&state.ordinary, &restored_name, &restored)?;
    state.parent.publish_file(
        &state.ordinary,
        OsStr::new(&restored_name),
        &restored_file,
        OsStr::new(&receipt.installed),
        Publication::New,
    )?;
    settle(state, receipt, false)
}

/// Effect-free pending-state hint only. Actual recovery revalidates retained
/// directories, bounded receipts and all image evidence; this grants no authority.
pub fn has_pending(parent: &Path) -> std::io::Result<bool> {
    for name in [RECEIPT, DRAFT] {
        match std::fs::symlink_metadata(parent.join(STATE).join(name)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Recover only existing installation state; never creates a state directory.
pub fn recover(parent: &Path) -> Result<Option<Outcome>> {
    let Some(state) = State::open(parent, false)? else {
        return Ok(None);
    };
    let Some(mut receipt) = load(&state.private, &state.parent)? else {
        return Ok(None);
    };
    recover_locked(&state, &mut receipt).map(Some)
}

/// Publish selected verified bytes. This function never executes any image.
pub fn transact(request: Request) -> Result<Outcome> {
    transact_observed(request, &mut |_| Ok(()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Checkpoint {
    PreparingSaved,
    CandidateWritten,
    BackupWritten,
    Prepared,
    PublishedBeforeReceipt,
    Published,
    BackupRemoved,
}

fn transact_observed(
    request: Request,
    observer: &mut dyn FnMut(Checkpoint) -> Result<()>,
) -> Result<Outcome> {
    request.installation.revalidate()?;
    let (parent, name, mut original_file) = request.installation.into_parts();
    ensure!(
        name == OsStr::new("kuru"),
        "Unix update requires the installed kuru name"
    );
    ensure!(
        request.target == crate::archive::host_target()?,
        "Unix update requires the native target"
    );
    if let Some(version) = &request.version {
        crate::archive::checked_version(version)?;
    }
    let state = State::open(parent.path(), true)?.context("update state was not opened")?;
    ensure!(
        state.parent.identity() == parent.identity(),
        "installation parent changed"
    );
    if let Some(mut receipt) = load(&state.private, &state.parent)? {
        recover_locked(&state, &mut receipt)?;
    }
    parent.require_owned_replacement(&name, &original_file)?;
    let original = image(&mut original_file)?;
    let operation = Uuid::new_v4().to_string();
    let mut receipt = Receipt {
        schema_version: 1,
        platform: "unix".into(),
        operation: operation.clone(),
        parent: parent.path().to_owned(),
        parent_identity: parent.identity().to_bytes(),
        installed: "kuru".into(),
        candidate: format!("candidate-{operation}"),
        backup: format!("backup-{operation}"),
        original: original.clone(),
        original_meta: Meta {
            version: None,
            target: request.target.clone(),
        },
        replacement: None,
        replacement_meta: Meta {
            version: request.version,
            target: request.target,
        },
        backup_image: None,
        restored: None,
        support_hashes: request
            .support
            .as_ref()
            .map(|files| std::array::from_fn(|index| crate::archive::digest(&files.0[index]))),
        phase: Phase::Preparing,
    };
    // The receipt precedes every attributable image creation.
    save(&state.private, &state.parent, &receipt)?;
    observer(Checkpoint::PreparingSaved)?;
    let bytes = candidate_bytes(request.candidate)?;
    receipt.replacement = Some(write_image(
        &state.ordinary,
        &receipt.candidate,
        &bytes,
        false,
    )?);
    observer(Checkpoint::CandidateWritten)?;
    receipt.backup_image = Some(copy_image(
        &state.private,
        &receipt.backup,
        &mut original_file,
        &original,
        true,
    )?);
    observer(Checkpoint::BackupWritten)?;
    receipt.phase = Phase::Prepared;
    save(&state.private, &state.parent, &receipt)?;
    observer(Checkpoint::Prepared)?;
    if let Some(files) = &request.support
        && let Err(error) = crate::shell_support::install_versioned(
            files,
            parent.path(),
            receipt
                .replacement_meta
                .version
                .as_deref()
                .context("shell support needs a release version")?,
            &receipt.replacement_meta.target,
        )
    {
        recover_locked(&state, &mut receipt)?;
        return Err(error).context("shell support could not be staged; executable unchanged");
    }
    // A retained preflight from before download/build grants no later overwrite.
    parent.require_owned_replacement(&name, &original_file)?;
    drop(verified(&parent, &receipt.installed, &original)?);
    let candidate = verified(
        &state.ordinary,
        &receipt.candidate,
        receipt.replacement.as_ref().expect("prepared replacement"),
    )?;
    if let Err(error) = parent.publish_file(
        &state.ordinary,
        OsStr::new(&receipt.candidate),
        &candidate,
        &name,
        Publication::ReplaceRegular,
    ) {
        let outcome = recover_locked(&state, &mut receipt)?;
        if outcome.published {
            return Ok(outcome);
        }
        return Err(error).context("update publication was rejected; executable unchanged");
    }
    observer(Checkpoint::PublishedBeforeReceipt)?;
    settle_observed(&state, &mut receipt, true, observer)
}

/// Maintainer fixture entrypoints only; ordinary Kuru never selects an observer.
#[cfg(feature = "tooling")]
pub mod test_support {
    use super::*;
    pub fn replace_observed(parent: &Path, candidate: &Path, checkpoint: &str) -> Result<Outcome> {
        let wanted = match checkpoint {
            "preparing" => Checkpoint::PreparingSaved,
            "candidate" => Checkpoint::CandidateWritten,
            "backup" => Checkpoint::BackupWritten,
            "prepared" => Checkpoint::Prepared,
            "publication" => Checkpoint::PublishedBeforeReceipt,
            "published" => Checkpoint::Published,
            "cleanup" => Checkpoint::BackupRemoved,
            _ => anyhow::bail!("unknown Unix fixture checkpoint"),
        };
        let request = Request {
            installation: crate::ownership::installed(
                &parent.join("kuru"),
                &crate::ownership::OwnershipEnv::default(),
            )?,
            candidate: Candidate::BuildInput(candidate.to_path_buf()),
            version: None,
            target: crate::archive::host_target()?.into(),
            support: None,
        };
        transact_observed(request, &mut |reached| {
            if reached == wanted {
                writeln!(
                    std::io::stdout().lock(),
                    "Unix update checkpoint {checkpoint}"
                )?;
                std::io::stdout().flush()?;
                let mut release = [0];
                std::io::stdin().read_exact(&mut release)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };

    const OLD: &[u8] = b"#!/bin/sh\necho never-execute-old\n";
    const NEW: &[u8] = b"#!/bin/sh\necho never-execute-new\n";

    fn installation() -> Result<(tempfile::TempDir, PathBuf)> {
        let fixture = tempfile::tempdir()?;
        let parent = fixture.path().canonicalize()?;
        fs::write(parent.join("kuru"), OLD)?;
        fs::set_permissions(parent.join("kuru"), fs::Permissions::from_mode(0o555))?;
        Ok((fixture, parent))
    }

    fn request(parent: &Path, candidate: Candidate) -> Result<Request> {
        Ok(Request {
            installation: crate::ownership::installed(
                &parent.join("kuru"),
                &crate::ownership::OwnershipEnv::default(),
            )?,
            candidate,
            version: Some("0.11.0".into()),
            target: crate::archive::host_target()?.into(),
            support: None,
        })
    }

    fn settled(parent: &Path, published: bool) -> Result<()> {
        assert_eq!(
            fs::read(parent.join("kuru"))?,
            if published { NEW } else { OLD }
        );
        let names = fs::read_dir(parent.join(STATE))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(names, vec![std::ffi::OsString::from(LOCK)]);
        assert!(recover(parent)?.is_none());
        Ok(())
    }

    #[test]
    fn unix_receipt_checkpoints_recover_exact_old_or_new_without_images() -> Result<()> {
        for checkpoint in [
            Checkpoint::PreparingSaved,
            Checkpoint::CandidateWritten,
            Checkpoint::BackupWritten,
            Checkpoint::Prepared,
            Checkpoint::PublishedBeforeReceipt,
            Checkpoint::Published,
            Checkpoint::BackupRemoved,
        ] {
            let (_fixture, parent) = installation()?;
            let result = transact_observed(
                request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                &mut |reached| {
                    if reached == checkpoint {
                        anyhow::bail!("fixture interruption at {reached:?}");
                    }
                    Ok(())
                },
            );
            assert!(
                result.is_err(),
                "checkpoint was not reached: {checkpoint:?}"
            );
            let outcome = recover(&parent)?.context("missing interrupted transaction")?;
            let published = matches!(
                checkpoint,
                Checkpoint::PublishedBeforeReceipt
                    | Checkpoint::Published
                    | Checkpoint::BackupRemoved
            );
            assert_eq!(outcome.published, published, "{checkpoint:?}");
            settled(&parent, published)?;
            let second = transact(request(&parent, Candidate::Bytes(NEW.to_vec()))?)?;
            assert!(second.published);
            settled(&parent, true)?;
        }
        Ok(())
    }

    #[test]
    fn unix_support_failure_keeps_published_image_and_retires_rollback_images() -> Result<()> {
        let (_fixture, parent) = installation()?;
        fs::create_dir(parent.join("share"))?;
        fs::set_permissions(parent.join("share"), fs::Permissions::from_mode(0o700))?;
        let unrelated = parent.join("foreign-man");
        fs::create_dir(&unrelated)?;
        fs::write(unrelated.join("sentinel"), b"unchanged")?;
        std::os::unix::fs::symlink(&unrelated, parent.join("share/man"))?;
        let mut input = request(&parent, Candidate::Bytes(NEW.to_vec()))?;
        input.support = Some(crate::shell_support::Files(std::array::from_fn(|index| {
            format!("support file {index}\n").into_bytes()
        })));
        let outcome = transact(input)?;
        assert!(outcome.published && outcome.support_incomplete);
        settled(&parent, true)?;
        assert_eq!(fs::read(unrelated.join("sentinel"))?, b"unchanged");
        Ok(())
    }

    #[test]
    fn unix_source_input_keeps_read_only_cargo_hardlinks_unchanged() -> Result<()> {
        let (_fixture, parent) = installation()?;
        let source = parent.join("build-input");
        fs::write(&source, NEW)?;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555))?;
        fs::hard_link(&source, parent.join("compiled-artifact"))?;
        let before = fs::metadata(&source)?;
        assert!(transact(request(&parent, Candidate::BuildInput(source.clone()))?)?.published);
        let after = fs::metadata(&source)?;
        assert_eq!(
            (
                before.dev(),
                before.ino(),
                before.nlink(),
                before.permissions().mode()
            ),
            (
                after.dev(),
                after.ino(),
                after.nlink(),
                after.permissions().mode()
            )
        );
        assert_eq!(fs::read(source)?, NEW);
        assert_eq!(fs::metadata(parent.join("kuru"))?.nlink(), 1);
        settled(&parent, true)
    }

    #[test]
    fn unix_missing_after_possible_publication_refuses_without_restoring_old() -> Result<()> {
        for checkpoint in [Checkpoint::PublishedBeforeReceipt, Checkpoint::Published] {
            let (_fixture, parent) = installation()?;
            assert!(
                transact_observed(
                    request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                    &mut |reached| {
                        if reached == checkpoint {
                            anyhow::bail!("fixture interruption");
                        }
                        Ok(())
                    }
                )
                .is_err()
            );
            fs::remove_file(parent.join("kuru"))?;
            let before = fs::read(parent.join(STATE).join(RECEIPT))?;
            let error = recover(&parent).expect_err("possible publication must refuse restoration");
            assert!(format!("{error:#}").contains("possible update publication"));
            assert_eq!(fs::read(parent.join(STATE).join(RECEIPT))?, before);
            assert!(!parent.join("kuru").exists());
        }
        Ok(())
    }

    #[test]
    fn unix_unpublished_missing_original_restores_only_from_candidate_proof() -> Result<()> {
        let (_fixture, parent) = installation()?;
        assert!(
            transact_observed(
                request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                &mut |reached| {
                    if reached == Checkpoint::Prepared {
                        anyhow::bail!("fixture interruption");
                    }
                    Ok(())
                }
            )
            .is_err()
        );
        fs::remove_file(parent.join("kuru"))?;
        assert!(
            !recover(&parent)?
                .context("missing prepared recovery")?
                .published
        );
        settled(&parent, false)
    }

    #[test]
    fn unix_restore_cleanup_with_original_present_keeps_unknown_evidence() -> Result<()> {
        for recorded in [false, true] {
            let (_fixture, parent) = installation()?;
            assert!(
                transact_observed(
                    request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                    &mut |reached| {
                        if reached == Checkpoint::Prepared {
                            anyhow::bail!("fixture interruption");
                        }
                        Ok(())
                    }
                )
                .is_err()
            );
            let state = State::open(&parent, false)?.context("missing state")?;
            let mut receipt = load(&state.private, &state.parent)?.context("missing receipt")?;
            receipt.phase = Phase::Restoring;
            save(&state.private, &state.parent, &receipt)?;
            let restored_name = format!("restored-{}", receipt.operation);
            if recorded {
                let mut original = verified(
                    &state.private,
                    &receipt.backup,
                    receipt.backup_image.as_ref().context("missing backup")?,
                )?;
                receipt.restored = Some(copy_image(
                    &state.ordinary,
                    &restored_name,
                    &mut original,
                    receipt.backup_image.as_ref().context("missing backup")?,
                    false,
                )?);
                save(&state.private, &state.parent, &receipt)?;
                let before = fs::read(parent.join(STATE).join(RECEIPT))?;
                fs::write(
                    parent.join(STATE).join(&restored_name),
                    b"unknown changed restore",
                )?;
                assert!(recover_locked(&state, &mut receipt).is_err());
                assert_eq!(fs::read(parent.join(STATE).join(RECEIPT))?, before);
                assert!(parent.join(STATE).join(&receipt.backup).exists());
                assert!(parent.join(STATE).join(&receipt.candidate).exists());
                fs::write(parent.join(STATE).join(&restored_name), OLD)?;
            } else {
                let mut partial = state.ordinary.create_new(OsStr::new(&restored_name))?;
                partial.write_all(&OLD[..5])?;
                partial.sync_all()?;
            }
            assert!(!recover_locked(&state, &mut receipt)?.published);
            drop(state);
            settled(&parent, false)?;
        }
        Ok(())
    }

    #[test]
    fn unix_receipt_invalid_or_busy_evidence_refuses_without_payload_effects() -> Result<()> {
        {
            let (_fixture, parent) = installation()?;
            let state = parent.join(STATE);
            fs::create_dir(&state)?;
            fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
            let orphan = state.join("candidate-without-intent");
            fs::write(&orphan, b"unattributed original-or-candidate")?;
            assert!(recover(&parent).is_err());
            assert!(transact(request(&parent, Candidate::Bytes(NEW.to_vec()))?).is_err());
            assert_eq!(fs::read(parent.join("kuru"))?, OLD);
            assert_eq!(fs::read(&orphan)?, b"unattributed original-or-candidate");
            assert!(!state.join(LOCK).exists());
            assert!(!state.join(RECEIPT).exists());
            // Legitimate empty and exact lock-only shapes remain usable.
            fs::remove_file(orphan)?;
            assert!(recover(&parent)?.is_none());
            assert!(transact(request(&parent, Candidate::Bytes(NEW.to_vec()))?)?.published);
            assert!(recover(&parent)?.is_none());
            assert!(transact(request(&parent, Candidate::Bytes(NEW.to_vec()))?)?.published);
        }

        for cause in [
            "schema",
            "parent",
            "digest",
            "linked-receipt",
            "linked-candidate",
            "state-mode",
            "busy",
        ] {
            let (_fixture, parent) = installation()?;
            assert!(
                transact_observed(
                    request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                    &mut |reached| {
                        if reached == Checkpoint::Prepared {
                            anyhow::bail!("fixture interruption");
                        }
                        Ok(())
                    }
                )
                .is_err()
            );
            let state = State::open(&parent, false)?.context("missing state")?;
            let mut receipt = load(&state.private, &state.parent)?.context("missing receipt")?;
            let receipt_path = parent.join(STATE).join(RECEIPT);
            match cause {
                "schema" => receipt.schema_version = 2,
                "parent" => receipt.parent_identity = [0; 24],
                "digest" => {
                    receipt
                        .replacement
                        .as_mut()
                        .context("missing candidate")?
                        .sha256 = "0".repeat(64)
                }
                "linked-receipt" => {
                    fs::hard_link(&receipt_path, parent.join("foreign-receipt-link"))?
                }
                "linked-candidate" => fs::hard_link(
                    parent.join(STATE).join(&receipt.candidate),
                    parent.join("foreign-candidate-link"),
                )?,
                "state-mode" => {
                    fs::set_permissions(parent.join(STATE), fs::Permissions::from_mode(0o755))?
                }
                "busy" => {}
                _ => unreachable!(),
            }
            if matches!(cause, "schema" | "parent" | "digest") {
                fs::write(&receipt_path, serde_json::to_vec(&receipt)?)?;
            }
            let before = fs::read(&receipt_path)?;
            // Busy retains this exact native lock; every other case releases
            // it so the actual invalid evidence, rather than a lock, refuses.
            let held = if cause == "busy" {
                Some(state)
            } else {
                drop(state);
                None
            };
            let error = recover(&parent).expect_err("invalid/busy evidence must refuse");
            if cause == "busy" {
                assert!(error.downcast_ref::<Busy>().is_some());
            }
            assert_eq!(fs::read(&receipt_path)?, before, "{cause}");
            assert_eq!(fs::read(parent.join("kuru"))?, OLD, "{cause}");
            assert_eq!(
                fs::read(parent.join(STATE).join(&receipt.candidate))?,
                NEW,
                "{cause}"
            );
            assert_eq!(
                fs::read(parent.join(STATE).join(&receipt.backup))?,
                OLD,
                "{cause}"
            );
            drop(held);
        }
        Ok(())
    }

    #[test]
    fn unix_receipt_draft_is_recoverable_only_when_complete_and_bound() -> Result<()> {
        let (_fixture, parent) = installation()?;
        assert!(
            transact_observed(
                request(&parent, Candidate::Bytes(NEW.to_vec()))?,
                &mut |reached| {
                    if reached == Checkpoint::PreparingSaved {
                        anyhow::bail!("fixture interruption");
                    }
                    Ok(())
                }
            )
            .is_err()
        );
        let state = parent.join(STATE);
        fs::rename(state.join(RECEIPT), state.join(DRAFT))?;
        assert!(
            !recover(&parent)?
                .context("complete initial draft must recover")?
                .published
        );
        settled(&parent, false)?;
        fs::write(state.join(DRAFT), b"{incomplete")?;
        fs::set_permissions(state.join(DRAFT), fs::Permissions::from_mode(0o600))?;
        assert!(recover(&parent).is_err());
        assert_eq!(fs::read(state.join(DRAFT))?, b"{incomplete");
        assert_eq!(fs::read(parent.join("kuru"))?, OLD);
        Ok(())
    }
}
