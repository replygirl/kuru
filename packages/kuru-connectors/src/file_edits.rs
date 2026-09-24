//! Exact, bounded source planning for native file edits.

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, copy_file_access, finalize_file_access,
    regular_file_info, verify_file_access, verify_retained_file_access,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path},
    sync::Arc,
};

const MAX_HUNKS: usize = 64;
const MAX_RECEIPT_BYTES: usize = 6 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const FORMAT: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEffect {
    Write,
    Edit,
    Delete,
    Undo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointState {
    Prepared,
    Applied,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareResult {
    New,
    Existing(CheckpointState),
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckpointSummary {
    pub id: String,
    pub path: String,
    pub effect: FileEffect,
    pub state: CheckpointState,
    pub undo_of: Option<String>,
    pub created: bool,
}

type CheckpointSnapshots = (CheckpointSummary, Option<Vec<u8>>, Option<Vec<u8>>);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    format: u8,
    project: String,
    project_identity: [u8; 24],
    // A checked parent identity binds later explicit stage discard to the
    // directory in which the private sibling was actually created.
    parent_identity: Option<[u8; 24]>,
    stage_identity: Option<[u8; 24]>,
    template_identity: Option<[u8; 24]>,
    id: String,
    fingerprint: String,
    path: String,
    effect: FileEffect,
    state: CheckpointState,
    undo_of: Option<String>,
    before: Option<String>,
    after: Option<String>,
    before_sha256: Option<String>,
    after_sha256: Option<String>,
    post_identity: Option<[u8; 24]>,
    // A Prepared receipt physically reserves the largest possible JSON
    // post_identity array. Settlement replaces this reservation, so capacity
    // can never fail solely because the target was already published.
    capacity_reservation: Option<[u8; 24]>,
}

impl Receipt {
    fn stage_reservation(&self) -> u64 {
        if matches!(
            self.state,
            CheckpointState::Prepared | CheckpointState::Uncertain
        ) {
            // Base64 is larger than the staged payload, so this conservatively
            // counts even an interrupted file that still exists in the tool root.
            self.after.as_ref().map_or(0, |bytes| bytes.len() as u64)
        } else {
            0
        }
    }
    fn summary(&self) -> CheckpointSummary {
        CheckpointSummary {
            id: self.id.clone(),
            path: self.path.clone(),
            effect: self.effect,
            state: self.state,
            undo_of: self.undo_of.clone(),
            created: self.before.is_none() && self.after.is_some(),
        }
    }

    fn bytes(&self, before: bool) -> Result<Option<Vec<u8>>> {
        let (encoded, digest) = if before {
            (&self.before, &self.before_sha256)
        } else {
            (&self.after, &self.after_sha256)
        };
        match (encoded, digest) {
            (None, None) => Ok(None),
            (Some(encoded), Some(digest)) => {
                ensure!(
                    encoded.len() <= crate::MAX_BYTES.div_ceil(3) * 4 + 4,
                    "checkpoint snapshot is oversized"
                );
                let bytes = STANDARD
                    .decode(encoded)
                    .context("checkpoint snapshot encoding is invalid")?;
                ensure!(
                    bytes.len() <= crate::MAX_BYTES && hash(&bytes) == *digest,
                    "checkpoint snapshot digest differs"
                );
                Ok(Some(bytes))
            }
            _ => anyhow::bail!("checkpoint snapshot metadata is incomplete"),
        }
    }
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn encoded(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(|value| STANDARD.encode(value))
}
fn digest(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(hash)
}
fn receipt_name(id: &str) -> Result<String> {
    ensure!(
        !id.is_empty() && id.len() <= 256,
        "checkpoint operation ID is invalid"
    );
    Ok(format!("receipt-{}.json", hash(id.as_bytes())))
}

fn project_digest(path: &Path) -> String {
    hash(path.as_os_str().as_encoded_bytes())
}

/// Private project-bound snapshots. The app supplies its already-selected data
/// directory; no model tool can address this directory as a workspace path.
pub struct CheckpointStore {
    directory: Directory,
    root: Arc<Directory>,
    project: String,
    project_identity: [u8; 24],
}

pub struct CheckpointLease<'a> {
    store: &'a CheckpointStore,
    lock: File,
}

struct TargetSnapshot {
    file: File,
    bytes: Vec<u8>,
    identity: kuru_platform::fs::FileIdentity,
}

fn target_snapshot(parent: &Directory, name: &OsStr) -> Result<Option<TargetSnapshot>> {
    let mut file = match parent.read(name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read checked file target"),
    };
    let info = regular_file_info(&file)?;
    ensure!(
        info.len <= crate::MAX_BYTES as u64,
        "file target exceeds 2 MiB snapshot limit"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= crate::MAX_BYTES,
        "file target grew past 2 MiB snapshot limit"
    );
    parent.verify(name, &file)?;
    Ok(Some(TargetSnapshot {
        file,
        bytes,
        identity: info.identity,
    }))
}

fn unchanged_target(
    parent: &Directory,
    name: &OsStr,
    captured: &mut Option<TargetSnapshot>,
) -> Result<()> {
    parent.revalidate()?;
    match captured {
        Some(target) => {
            parent.verify(name, &target.file)?;
            let info = regular_file_info(&target.file)?;
            ensure!(
                info.identity == target.identity,
                "file target identity changed"
            );
            target.file.seek(SeekFrom::Start(0))?;
            let mut current = Vec::new();
            (&mut target.file)
                .take(crate::MAX_BYTES as u64 + 1)
                .read_to_end(&mut current)?;
            ensure!(
                current == target.bytes,
                "file target changed before publication"
            );
            parent.verify(name, &target.file)?;
        }
        None => match parent.read(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => anyhow::bail!("file target appeared before publication"),
            Err(error) => return Err(error.into()),
        },
    }
    Ok(())
}

impl CheckpointStore {
    pub fn new(data: &Path, root: Arc<Directory>) -> Result<Self> {
        Self::open(data, root, true)?.context("created file checkpoint store is missing")
    }

    /// Read-only entrypoint for inspection. An untouched project has no
    /// checkpoint directory, and inspecting it must not create one.
    pub fn existing(data: &Path, root: Arc<Directory>) -> Result<Option<Self>> {
        Self::open(data, root, false)
    }

    fn open(data: &Path, root: Arc<Directory>, create: bool) -> Result<Option<Self>> {
        root.revalidate()?;
        let data = if create {
            Directory::ensure_private(data)
                .context("file checkpoint data directory is not owner-private")?
        } else {
            match Directory::open(data, Privacy::OwnerOnly, NameRetention::Movable) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(error).context("open private file checkpoint data directory");
                }
            }
        };
        ensure!(
            !data.is_within(&root)? && !root.is_within(&data)?,
            "file checkpoint data directory overlaps tool root"
        );
        let project = project_digest(root.path());
        let project_identity = root.identity().to_bytes();
        let name = format!("file-checkpoints-{project}");
        let path = data.path().join(&name);
        let directory = match Directory::open(&path, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match data.create_private_directory(OsStr::new(&name)) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        Directory::open(&path, Privacy::OwnerOnly, NameRetention::Movable)?
                    }
                    Err(error) => {
                        return Err(error).context("create private file checkpoint directory");
                    }
                }
            }
            Err(error) => return Err(error).context("open private file checkpoint directory"),
        };
        directory.revalidate()?;
        Ok(Some(Self {
            directory,
            root,
            project,
            project_identity,
        }))
    }

    pub fn lease(&self) -> Result<CheckpointLease<'_>> {
        self.root.revalidate()?;
        let lock = self.directory.lock_file(OsStr::new("checkpoint.lock"))?;
        lock.try_lock().context("file checkpoint store is busy")?;
        self.directory
            .verify(OsStr::new("checkpoint.lock"), &lock)?;
        Ok(CheckpointLease { store: self, lock })
    }

    pub fn validate_root(&self, root: &Directory) -> Result<()> {
        root.revalidate()?;
        ensure!(
            project_digest(root.path()) == self.project
                && root.identity().to_bytes() == self.project_identity,
            "checkpoint store belongs to another project root"
        );
        self.root.revalidate()?;
        Ok(())
    }

    pub fn inspect(&self, id: &str) -> Result<Option<CheckpointSummary>> {
        self.lease()?.inspect(id)
    }

    pub fn list(&self, limit: usize) -> Result<Vec<CheckpointSummary>> {
        self.lease()?.list(limit)
    }

    pub fn prune(&self, id: &str, discard_uncertain: bool) -> Result<bool> {
        self.lease()?.prune(id, discard_uncertain)
    }
}

impl CheckpointLease<'_> {
    fn verify(&self) -> Result<()> {
        self.store.root.revalidate()?;
        self.store
            .directory
            .verify(OsStr::new("checkpoint.lock"), &self.lock)?;
        Ok(())
    }

    fn read(&self, id: &str) -> Result<Option<Receipt>> {
        self.verify()?;
        let name = receipt_name(id)?;
        let mut file = match self.store.directory.read(OsStr::new(&name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read file checkpoint receipt"),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_RECEIPT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RECEIPT_BYTES,
            "checkpoint receipt exceeds bound"
        );
        self.store.directory.verify(OsStr::new(&name), &file)?;
        let record: Receipt =
            serde_json::from_slice(&bytes).context("decode file checkpoint receipt")?;
        ensure!(
            record.format == FORMAT
                && record.project == self.store.project
                && record.project_identity == self.store.project_identity
                && record.id == id,
            "checkpoint receipt binding differs"
        );
        ensure!(
            record.bytes(true)?.is_some() == record.before.is_some()
                && record.bytes(false)?.is_some() == record.after.is_some(),
            "checkpoint receipt snapshot invalid"
        );
        ensure!(
            record.fingerprint.len() == 64
                && record
                    .fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "checkpoint request fingerprint is invalid"
        );
        ensure!(
            match record.effect {
                FileEffect::Write => record.after.is_some(),
                FileEffect::Edit => record.before.is_some() && record.after.is_some(),
                FileEffect::Delete => record.before.is_some() && record.after.is_none(),
                FileEffect::Undo => record.undo_of.is_some(),
            },
            "checkpoint effect snapshots are inconsistent"
        );
        ensure!(
            (record.state == CheckpointState::Applied && record.after.is_some())
                == record.post_identity.is_some(),
            "checkpoint post-publication identity is inconsistent"
        );
        ensure!(
            record.capacity_reservation
                == (record.state == CheckpointState::Prepared).then_some([u8::MAX; 24]),
            "checkpoint capacity reservation is inconsistent"
        );
        ensure!(
            if record.parent_identity.is_some() {
                record.stage_identity.is_some() == record.after.is_some()
                    && record.template_identity.is_some()
                        == (record.after.is_some() && record.before.is_none())
            } else {
                record.stage_identity.is_none() && record.template_identity.is_none()
            },
            "checkpoint staged-object binding is inconsistent"
        );
        ensure!(
            matches!(
                Path::new(&record.path).components().next(),
                Some(Component::Normal(_))
            ) && Path::new(&record.path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "checkpoint path is invalid"
        );
        Ok(Some(record))
    }

    fn occupied_bytes(&self) -> Result<u64> {
        self.verify()?;
        let mut total = 0u64;
        for (count, entry) in std::fs::read_dir(self.store.directory.path())?.enumerate() {
            ensure!(
                count < MAX_ENTRIES,
                "checkpoint inventory exceeds entry bound"
            );
            let name = entry?.file_name();
            let mut file = self.store.directory.read(&name)?;
            let info = regular_file_info(&file)?;
            let mut occupied = info.len;
            if name
                .to_str()
                .is_some_and(|name| name.starts_with("receipt-") && name.ends_with(".json"))
            {
                ensure!(
                    info.len <= MAX_RECEIPT_BYTES as u64,
                    "checkpoint receipt exceeds bound"
                );
                let mut bytes = Vec::new();
                (&mut file)
                    .take(MAX_RECEIPT_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 == info.len,
                    "checkpoint inventory receipt changed during capacity check"
                );
                self.store.directory.verify(&name, &file)?;
                let record: Receipt = serde_json::from_slice(&bytes)
                    .context("decode checkpoint inventory receipt")?;
                ensure!(
                    receipt_name(&record.id)? == name.to_string_lossy()
                        && record.project == self.store.project
                        && record.project_identity == self.store.project_identity,
                    "checkpoint inventory binding differs"
                );
                occupied = occupied
                    .checked_add(record.stage_reservation())
                    .context("checkpoint stage reservation overflow")?;
            }
            total = total
                .checked_add(occupied)
                .context("checkpoint inventory size overflow")?;
            ensure!(
                total <= MAX_TOTAL_BYTES,
                "checkpoint inventory exceeds 128 MiB limit"
            );
        }
        Ok(total)
    }

    fn write(&self, record: &Receipt) -> Result<()> {
        self.verify()?;
        let bytes = serde_json::to_vec(record)?;
        ensure!(
            bytes.len() <= MAX_RECEIPT_BYTES,
            "checkpoint receipt exceeds bound"
        );
        let name = receipt_name(&record.id)?;
        let old_len = match self.store.directory.read(OsStr::new(&name)) {
            Ok(file) => {
                regular_file_info(&file)?.len
                    + self
                        .read(&record.id)?
                        .context("checkpoint receipt vanished")?
                        .stage_reservation()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let used = self.occupied_bytes()?;
        let next = used
            .checked_sub(old_len)
            .and_then(|size| size.checked_add(bytes.len() as u64))
            .and_then(|size| size.checked_add(record.stage_reservation()))
            .context("checkpoint inventory changed during capacity check")?;
        ensure!(
            next <= MAX_TOTAL_BYTES,
            "file checkpoints are full; explicitly prune a settled receipt before another edit"
        );
        let temporary = format!("receipt-{}.tmp", uuid::Uuid::new_v4());
        let mut file = self.store.directory.create_new(OsStr::new(&temporary))?;
        file.write_all(&bytes)?;
        self.store
            .directory
            .publish_file(
                &self.store.directory,
                OsStr::new(&temporary),
                &file,
                OsStr::new(&name),
                Publication::ReplaceRegular,
            )
            .context("publish file checkpoint receipt; uncertain private candidate remains")?;
        Ok(())
    }

    pub fn inspect(&self, id: &str) -> Result<Option<CheckpointSummary>> {
        Ok(self.read(id)?.map(|r| r.summary()))
    }

    pub fn list(&self, limit: usize) -> Result<Vec<CheckpointSummary>> {
        ensure!(
            (1..=1000).contains(&limit),
            "checkpoint list limit must be 1–1000"
        );
        self.verify()?;
        let mut records = Vec::new();
        for (count, entry) in std::fs::read_dir(self.store.directory.path())?.enumerate() {
            ensure!(
                count < MAX_ENTRIES,
                "checkpoint inventory exceeds entry bound"
            );
            let name = entry?.file_name();
            let name = name.to_str().context("checkpoint filename is not UTF-8")?;
            if !name.starts_with("receipt-") || !name.ends_with(".json") {
                continue;
            }
            let mut file = self.store.directory.read(OsStr::new(name))?;
            let mut bytes = Vec::new();
            (&mut file)
                .take(MAX_RECEIPT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= MAX_RECEIPT_BYTES,
                "checkpoint receipt exceeds bound"
            );
            self.store.directory.verify(OsStr::new(name), &file)?;
            let candidate: Receipt = serde_json::from_slice(&bytes)?;
            ensure!(
                receipt_name(&candidate.id)? == name,
                "checkpoint filename does not match receipt ID"
            );
            let record = self
                .read(&candidate.id)?
                .context("checkpoint disappeared during listing")?;
            records.push(record.summary());
        }
        records.sort_by(|a, b| a.id.cmp(&b.id));
        records.truncate(limit);
        Ok(records)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "durable receipt binds the exact request and auxiliary object identities"
    )]
    pub fn prepare(
        &self,
        id: &str,
        fingerprint: &str,
        path: &str,
        effect: FileEffect,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
        undo_of: Option<&str>,
        parent_identity: Option<[u8; 24]>,
        stage_identity: Option<[u8; 24]>,
        template_identity: Option<[u8; 24]>,
    ) -> Result<PrepareResult> {
        receipt_name(id)?;
        ensure!(
            fingerprint.len() == 64 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "checkpoint request fingerprint is invalid"
        );
        ensure!(
            matches!(
                Path::new(path).components().next(),
                Some(Component::Normal(_))
            ) && Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "checkpoint path is invalid"
        );
        ensure!(
            before.is_none_or(|b| b.len() <= crate::MAX_BYTES)
                && after.is_none_or(|b| b.len() <= crate::MAX_BYTES),
            "checkpoint snapshot exceeds 2 MiB limit"
        );
        let mut record = Receipt {
            format: FORMAT,
            project: self.store.project.clone(),
            project_identity: self.store.project_identity,
            parent_identity,
            stage_identity,
            template_identity,
            id: id.to_owned(),
            fingerprint: fingerprint.to_owned(),
            path: path.to_owned(),
            effect,
            state: CheckpointState::Prepared,
            undo_of: undo_of.map(str::to_owned),
            before: encoded(before),
            after: encoded(after),
            before_sha256: digest(before),
            after_sha256: digest(after),
            post_identity: None,
            capacity_reservation: Some([u8::MAX; 24]),
        };
        if let Some(existing) = self.read(id)? {
            record.state = existing.state;
            record.post_identity = existing.post_identity;
            record.capacity_reservation = existing.capacity_reservation;
            ensure!(
                serde_json::to_vec(&existing)? == serde_json::to_vec(&record)?,
                "checkpoint operation ID conflicts with prior arguments"
            );
            return Ok(PrepareResult::Existing(existing.state));
        }
        self.write(&record)?;
        Ok(PrepareResult::New)
    }

    pub fn settle(
        &self,
        id: &str,
        state: CheckpointState,
        post_identity: Option<[u8; 24]>,
    ) -> Result<()> {
        ensure!(
            state != CheckpointState::Prepared,
            "cannot settle checkpoint as prepared"
        );
        let mut record = self.read(id)?.context("checkpoint receipt is missing")?;
        ensure!(
            record.state == CheckpointState::Prepared || record.state == state,
            "checkpoint receipt already settled differently"
        );
        record.state = state;
        record.post_identity = post_identity;
        record.capacity_reservation = None;
        self.write(&record)
    }

    pub fn snapshots(&self, id: &str) -> Result<Option<CheckpointSnapshots>> {
        self.read(id)?
            .map(|record| Ok((record.summary(), record.bytes(true)?, record.bytes(false)?)))
            .transpose()
    }

    pub fn prune(&self, id: &str, discard_uncertain: bool) -> Result<bool> {
        let Some(record) = self.read(id)? else {
            return Ok(false);
        };
        ensure!(
            record.state == CheckpointState::Applied || discard_uncertain,
            "uncertain checkpoint requires explicit discard"
        );
        if record.after.is_some()
            && let Some(expected_parent) = record.parent_identity
        {
            let relative_parent = Path::new(&record.path)
                .parent()
                .context("checkpoint target has no parent")?;
            let parent_path = self.store.root.path().join(relative_parent);
            let parent = Directory::open(&parent_path, Privacy::Inherited, NameRetention::Movable)
                .context("reopen original checkpoint target parent for stage discard")?;
            ensure!(
                parent.is_within(&self.store.root)?
                    && parent.identity().to_bytes() == expected_parent,
                "checkpoint target parent changed; private stage cannot be safely discarded"
            );
            let stage_name = format!(".kuru-edit-{}", hash(id.as_bytes()));
            match Directory::open(
                &parent_path.join(&stage_name),
                Privacy::OwnerOnly,
                NameRetention::Movable,
            ) {
                Ok(stage) => {
                    ensure!(
                        stage.is_within(&parent)?
                            && Some(stage.identity().to_bytes()) == record.stage_identity,
                        "checkpoint stage identity changed; refusing to discard another directory"
                    );
                    stage
                        .remove_tree()
                        .context("remove selected private checkpoint stage")?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("open selected private checkpoint stage"),
            }
            if record.before.is_none() {
                let template_name = format!(".kuru-edit-template-{}", hash(id.as_bytes()));
                match parent.read(OsStr::new(&template_name)) {
                    Ok(template) => {
                        let info = regular_file_info(&template)?;
                        ensure!(
                            info.len == 0
                                && info.links == 1
                                && Some(info.identity.to_bytes()) == record.template_identity,
                            "checkpoint access template identity or contents changed"
                        );
                        parent
                            .remove_file(OsStr::new(&template_name), template)
                            .context("remove selected empty checkpoint access template")?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("open checkpoint access template"),
                }
            }
        }
        let name = receipt_name(id)?;
        let file = self.store.directory.read(OsStr::new(&name))?;
        self.store
            .directory
            .remove_file(OsStr::new(&name), file)
            .context("remove selected checkpoint receipt")?;
        Ok(true)
    }

    /// Perform one admitted mutation while holding the project checkpoint lock.
    /// A prepared receipt is durable before workspace publication, and a prior
    /// prepared result is never replayed from a matching pathname alone.
    #[expect(
        clippy::too_many_arguments,
        reason = "checked file mutation keeps authority, receipt and target explicit"
    )]
    pub fn mutate(
        &self,
        id: &str,
        fingerprint: &str,
        path: &str,
        effect: FileEffect,
        parent: &Directory,
        name: &OsStr,
        change: impl FnOnce(Option<&[u8]>) -> Result<Option<Vec<u8>>>,
    ) -> Result<CheckpointSummary> {
        self.mutate_inner(
            id,
            fingerprint,
            path,
            effect,
            None,
            None,
            parent,
            name,
            change,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "undo adds expected identity to the same checked mutation path"
    )]
    fn mutate_inner(
        &self,
        id: &str,
        fingerprint: &str,
        path: &str,
        effect: FileEffect,
        undo_of: Option<&str>,
        expected_identity: Option<Option<[u8; 24]>>,
        parent: &Directory,
        name: &OsStr,
        change: impl FnOnce(Option<&[u8]>) -> Result<Option<Vec<u8>>>,
    ) -> Result<CheckpointSummary> {
        self.verify()?;
        ensure!(
            parent.is_within(&self.store.root)?,
            "file target parent is outside checked project root"
        );
        if let Some(existing) = self.read(id)? {
            ensure!(
                existing.fingerprint == fingerprint
                    && existing.path == path
                    && existing.effect == effect
                    && existing.undo_of.as_deref() == undo_of,
                "checkpoint operation ID conflicts with prior arguments"
            );
            ensure!(
                existing.state == CheckpointState::Applied,
                "file checkpoint remains uncertain; inspect or explicitly discard its inactive receipt"
            );
            let current = target_snapshot(parent, name)?;
            ensure!(
                current.as_ref().map(|snapshot| snapshot.bytes.as_slice())
                    == existing.bytes(false)?.as_deref()
                    && current
                        .as_ref()
                        .map(|snapshot| snapshot.identity.to_bytes())
                        == existing.post_identity,
                "applied checkpoint post-state changed"
            );
            return Ok(existing.summary());
        }
        let mut before = target_snapshot(parent, name)?;
        if let Some(expected) = expected_identity {
            ensure!(
                before.as_ref().map(|snapshot| snapshot.identity.to_bytes()) == expected,
                "file target identity changed since selected checkpoint"
            );
        }
        let after = change(before.as_ref().map(|snapshot| snapshot.bytes.as_slice()))?;
        ensure!(
            after
                .as_ref()
                .is_none_or(|bytes| bytes.len() <= crate::MAX_BYTES),
            "file result exceeds 2 MiB snapshot limit"
        );
        // Create only empty auxiliary objects before the durable receipt.
        // Their exact native identities are captured in that receipt before
        // any private payload is staged, so later discard never selects a
        // replacement solely by a predictable pathname.
        let stage_name = format!(".kuru-edit-{}", hash(id.as_bytes()));
        let template_name = format!(".kuru-edit-template-{}", hash(id.as_bytes()));
        let mut stage = if after.is_some() {
            Some(parent.create_private_directory(OsStr::new(&stage_name))?)
        } else {
            None
        };
        let mut template = if after.is_some() && before.is_none() {
            match parent.create_new(OsStr::new(&template_name)) {
                Ok(template) => Some(template),
                Err(error) => {
                    if let Some(stage) = stage.take() {
                        let _ = stage.remove_tree();
                    }
                    return Err(error).context("create empty file-access template");
                }
            }
        } else {
            None
        };
        let stage_identity = stage.as_ref().map(|stage| stage.identity().to_bytes());
        let template_identity = template
            .as_ref()
            .map(|template| regular_file_info(template).map(|info| info.identity.to_bytes()))
            .transpose()?;
        let prior = self.prepare(
            id,
            fingerprint,
            path,
            effect,
            before.as_ref().map(|snapshot| snapshot.bytes.as_slice()),
            after.as_deref(),
            undo_of,
            Some(parent.identity().to_bytes()),
            stage_identity,
            template_identity,
        );
        let prior = match prior {
            Ok(prior) => prior,
            Err(error) => {
                if let Some(template) = template.take() {
                    let _ = parent.remove_file(OsStr::new(&template_name), template);
                }
                if let Some(stage) = stage.take() {
                    let _ = stage.remove_tree();
                }
                return Err(error);
            }
        };
        ensure!(
            prior == PrepareResult::New,
            "checkpoint operation was prepared concurrently"
        );
        unchanged_target(parent, name, &mut before)?;
        let post_identity = match after {
            Some(bytes) => {
                // The payload remains behind a private directory even after
                // its file receives the ordinary target ACL/mode. A replacement
                // copies its exact existing access policy; a create obtains the
                // checked parent's inherited policy from an empty template.
                let stage = stage.expect("staged result has a bound private directory");
                let mut candidate = stage.create_new(OsStr::new("payload"))?;
                candidate.write_all(&bytes)?;
                let access_source = before
                    .as_ref()
                    .map(|original| &original.file)
                    .or(template.as_ref())
                    .expect("replace has an original; create has an empty template");
                let access_token = copy_file_access(access_source, &candidate)?;
                stage.revalidate()?;
                unchanged_target(parent, name, &mut before)?;
                // The source view permits the staged file's copied ACL, while
                // the retained owner-private stage proves traversal shielding.
                let source =
                    Directory::open(stage.path(), Privacy::Inherited, NameRetention::Movable)?;
                ensure!(
                    source.identity() == stage.identity(),
                    "private stage directory changed"
                );
                let policy = if before.is_some() {
                    Publication::ReplaceRegular
                } else {
                    Publication::New
                };
                let access_source = before
                    .as_ref()
                    .map(|original| &original.file)
                    .or(template.as_ref())
                    .expect("replace has an original; create has an empty template");
                verify_file_access(access_source, &access_token)?;
                if let Err(error) = parent.publish_file_with_access(
                    &source,
                    OsStr::new("payload"),
                    &candidate,
                    &access_token,
                    name,
                    policy,
                ) {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error).context(
                        "file publication is unresolved; checkpoint retained for inspection",
                    );
                }
                if let Err(error) = finalize_file_access(access_source, &candidate) {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error).context(
                        "file was published but final access policy is unresolved; checkpoint retained",
                    );
                }
                if let Err(error) = verify_retained_file_access(access_source, &access_token) {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error).context(
                        "file was published while source access changed; checkpoint retained",
                    );
                }
                if let Some(template) = template
                    && let Err(error) = parent.remove_file(OsStr::new(&template_name), template)
                {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error).context(
                        "file was published but empty access template cleanup is unresolved",
                    );
                }
                if let Err(error) = parent.verify(name, &candidate) {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error).context("published file identity is unresolved");
                }
                let identity = match regular_file_info(&candidate) {
                    Ok(info) => info.identity.to_bytes(),
                    Err(error) => {
                        let _ = self.settle(id, CheckpointState::Uncertain, None);
                        return Err(error).context("published file identity is unresolved");
                    }
                };
                drop(source);
                drop(candidate);
                // A failed cleanup leaves only an empty private tool-hidden
                // directory; the applied receipt and target remain authoritative.
                let _ = stage.remove_tree();
                Some(identity)
            }
            None => {
                let original = before.context("file delete requires an existing regular file")?;
                if let Err(error) = parent.remove_file(name, original.file) {
                    let _ = self.settle(id, CheckpointState::Uncertain, None);
                    return Err(error)
                        .context("file removal is unresolved; checkpoint retained for inspection");
                }
                None
            }
        };
        #[cfg(test)]
        INTERRUPT_AFTER_EFFECT.with(|interrupt| {
            if interrupt.replace(false) {
                panic!("test interruption after file effect, before Applied receipt");
            }
        });
        self.settle(id, CheckpointState::Applied, post_identity)
            .context("file effect published, but checkpoint settlement is uncertain")?;
        self.inspect(id)?.context("applied checkpoint vanished")
    }

    /// A selected undo has its own deterministic receipt. A completed retry
    /// checks that undo's post-state, while a prepared undo remains uncertain.
    pub fn undo(
        &self,
        original_id: &str,
        parent: &Directory,
        name: &OsStr,
    ) -> Result<CheckpointSummary> {
        let original = self
            .read(original_id)?
            .context("selected file checkpoint does not exist")?;
        ensure!(
            original.state == CheckpointState::Applied && original.effect != FileEffect::Undo,
            "only an applied file effect can be undone"
        );
        let expected_post = original.bytes(false)?;
        let restore = original.bytes(true)?;
        let undo_id = format!("undo-{}", hash(original_id.as_bytes()));
        let fingerprint = hash(format!("undo:{}:{}", original_id, original.fingerprint).as_bytes());
        self.mutate_inner(
            &undo_id,
            &fingerprint,
            &original.path,
            FileEffect::Undo,
            Some(original_id),
            Some(original.post_identity),
            parent,
            name,
            |current| {
                ensure!(
                    current == expected_post.as_deref(),
                    "file post-state changed since selected checkpoint"
                );
                Ok(restore)
            },
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditHunk {
    pub before: String,
    pub old: String,
    pub after: String,
    pub replacement: String,
}

/// Plan every hunk against the same source snapshot. Nothing is published until
/// every match is unique, ordered, nonoverlapping and within the file bound.
pub(crate) fn apply_hunks(source: &[u8], hunks: &[EditHunk]) -> Result<Vec<u8>> {
    ensure!(
        source.len() <= crate::MAX_BYTES,
        "file source exceeds 2 MiB edit limit"
    );
    let source = std::str::from_utf8(source).context("file_edit requires UTF-8 source")?;
    ensure!(
        !hunks.is_empty() && hunks.len() <= MAX_HUNKS,
        "file_edit requires 1–64 hunks"
    );
    let mut output = Vec::with_capacity(source.len());
    let mut copied_through = 0;
    let mut previous_match_end = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        ensure!(
            !hunk.before.is_empty() || !hunk.old.is_empty() || !hunk.after.is_empty(),
            "file_edit hunk {} has no source context",
            index + 1
        );
        let pattern_len = hunk
            .before
            .len()
            .checked_add(hunk.old.len())
            .and_then(|n| n.checked_add(hunk.after.len()));
        ensure!(
            pattern_len.is_some_and(|n| n <= crate::MAX_BYTES),
            "file_edit hunk {} exceeds source limit",
            index + 1
        );
        let pattern = format!("{}{}{}", hunk.before, hunk.old, hunk.after);
        let mut matches = source.match_indices(&pattern);
        let Some((match_start, _)) = matches.next() else {
            anyhow::bail!("file_edit hunk {} is stale", index + 1);
        };
        ensure!(
            matches.next().is_none(),
            "file_edit hunk {} is ambiguous",
            index + 1
        );
        let match_end = match_start + pattern.len();
        ensure!(
            match_start >= previous_match_end,
            "file_edit hunk {} overlaps or precedes another hunk",
            index + 1
        );
        let old_start = match_start + hunk.before.len();
        let old_end = old_start + hunk.old.len();
        let projected = output
            .len()
            .checked_add(old_start - copied_through)
            .and_then(|size| size.checked_add(hunk.replacement.len()));
        ensure!(
            projected.is_some_and(|size| size <= crate::MAX_BYTES),
            "file_edit output exceeds 2 MiB limit"
        );
        output.extend_from_slice(&source.as_bytes()[copied_through..old_start]);
        output.extend_from_slice(hunk.replacement.as_bytes());
        copied_through = old_end;
        previous_match_end = match_end;
    }
    ensure!(
        output
            .len()
            .checked_add(source.len() - copied_through)
            .is_some_and(|size| size <= crate::MAX_BYTES),
        "file_edit output exceeds 2 MiB limit"
    );
    output.extend_from_slice(&source.as_bytes()[copied_through..]);
    Ok(output)
}

#[cfg(test)]
thread_local! {
    static INTERRUPT_AFTER_EFFECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn checked_receipts_support_restart_undo_but_never_infer_a_prepared_create() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let project = temporary.path().join("project");
        std::fs::create_dir(&project)?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable)?;
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let name = OsStr::new("note.txt");
        let store = CheckpointStore::new(private.path(), root.clone())?;
        {
            let lease = store.lease()?;
            let created = lease.mutate(
                "create-1",
                &hash(b"create-1-args"),
                "note.txt",
                FileEffect::Write,
                &parent,
                name,
                |_| Ok(Some(b"before".to_vec())),
            )?;
            ensure!(
                created.state == CheckpointState::Applied,
                "create did not settle"
            );
        }
        drop(store);
        let reopened = CheckpointStore::new(private.path(), root.clone())?;
        {
            let lease = reopened.lease()?;
            lease.mutate(
                "replace-1",
                &hash(b"replace-1-args"),
                "note.txt",
                FileEffect::Write,
                &parent,
                name,
                |_| Ok(Some(b"after".to_vec())),
            )?;
            lease.undo("replace-1", &parent, name)?;
            ensure!(
                std::fs::read(project.join("note.txt"))? == b"before",
                "replacement undo did not restore source"
            );
            lease.mutate(
                "delete-1",
                &hash(b"delete-1-args"),
                "note.txt",
                FileEffect::Delete,
                &parent,
                name,
                |_| Ok(None),
            )?;
        }
        drop(reopened);
        let reopened = CheckpointStore::new(private.path(), root.clone())?;
        {
            let lease = reopened.lease()?;
            lease.undo("delete-1", &parent, name)?;
            ensure!(
                std::fs::read(project.join("note.txt"))? == b"before",
                "delete undo did not restore source"
            );
            lease.prepare(
                "prepared-foreign",
                &hash(b"prepared-foreign-args"),
                "same.txt",
                FileEffect::Write,
                None,
                Some(b"same"),
                None,
                None,
                None,
                None,
            )?;
        }
        std::fs::write(project.join("same.txt"), b"same")?;
        let lease = reopened.lease()?;
        let error = lease
            .mutate(
                "prepared-foreign",
                &hash(b"prepared-foreign-args"),
                "same.txt",
                FileEffect::Write,
                &parent,
                OsStr::new("same.txt"),
                |_| Ok(Some(b"same".to_vec())),
            )
            .unwrap_err();
        ensure!(
            error.to_string().contains("uncertain"),
            "prepared matching content was treated as applied: {error:#}"
        );
        ensure!(
            std::fs::read(project.join("same.txt"))? == b"same",
            "prepared retry changed foreign content"
        );
        lease.prepare(
            "prepared-delete",
            &hash(b"prepared-delete-args"),
            "note.txt",
            FileEffect::Delete,
            Some(b"before"),
            None,
            None,
            None,
            None,
            None,
        )?;
        std::fs::remove_file(project.join("note.txt"))?;
        let error = lease
            .mutate(
                "prepared-delete",
                &hash(b"prepared-delete-args"),
                "note.txt",
                FileEffect::Delete,
                &parent,
                name,
                |_| Ok(None),
            )
            .unwrap_err();
        ensure!(
            error.to_string().contains("uncertain"),
            "prepared delete absence was treated as proof: {error:#}"
        );
        ensure!(
            !project.join("note.txt").exists(),
            "prepared delete retry recreated or changed the target"
        );
        lease.mutate(
            "create-undo",
            &hash(b"create-undo-args"),
            "created.txt",
            FileEffect::Write,
            &parent,
            OsStr::new("created.txt"),
            |_| Ok(Some(b"created".to_vec())),
        )?;
        std::fs::write(project.join("user.txt"), b"before")?;
        lease.mutate(
            "replace-user",
            &hash(b"replace-user-args"),
            "user.txt",
            FileEffect::Write,
            &parent,
            OsStr::new("user.txt"),
            |_| Ok(Some(b"after".to_vec())),
        )?;
        // An external in-place edit keeps the native identity but changes the
        // checked post bytes; selected undo must preserve that user work.
        std::fs::write(project.join("user.txt"), b"user changed")?;
        drop(lease);
        drop(reopened);
        let reopened = CheckpointStore::new(private.path(), root)?;
        let lease = reopened.lease()?;
        lease.undo("create-undo", &parent, OsStr::new("created.txt"))?;
        ensure!(
            !project.join("created.txt").exists(),
            "reopened create undo left its target"
        );
        let conflict = lease
            .undo("replace-user", &parent, OsStr::new("user.txt"))
            .unwrap_err();
        ensure!(
            conflict.to_string().contains("changed"),
            "external user edit did not conflict with selected undo: {conflict:#}"
        );
        ensure!(
            std::fs::read(project.join("user.txt"))? == b"user changed",
            "selected undo overwrote an external user edit"
        );
        Ok(())
    }

    #[test]
    fn interrupted_real_file_effect_stays_prepared_after_reopen() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let project = temporary.path().join("project");
        std::fs::create_dir(&project)?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable)?;
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let cases = [
            ("create", None, Some(&b"created"[..])),
            ("replace", Some(&b"before"[..]), Some(&b"after"[..])),
            ("delete", Some(&b"before"[..]), None),
        ];
        for (id, before, after) in cases {
            let relative = format!("{id}.txt");
            let path = project.join(&relative);
            if let Some(bytes) = before {
                std::fs::write(&path, bytes)?;
            }
            let effect = if after.is_none() {
                FileEffect::Delete
            } else {
                FileEffect::Write
            };
            let fingerprint = hash(id.as_bytes());
            let store = CheckpointStore::new(private.path(), root.clone())?;
            let lease = store.lease()?;
            INTERRUPT_AFTER_EFFECT.with(|interrupt| interrupt.set(true));
            let interrupted = catch_unwind(AssertUnwindSafe(|| {
                lease.mutate(
                    id,
                    &fingerprint,
                    &relative,
                    effect,
                    &parent,
                    OsStr::new(&relative),
                    |_| Ok(after.map(<[u8]>::to_vec)),
                )
            }));
            INTERRUPT_AFTER_EFFECT.with(|interrupt| interrupt.set(false));
            ensure!(interrupted.is_err(), "{id} did not reach the interruption");
            drop(lease);
            drop(store);

            let reopened = CheckpointStore::new(private.path(), root.clone())?;
            let lease = reopened.lease()?;
            ensure!(
                lease
                    .inspect(id)?
                    .is_some_and(|summary| summary.state == CheckpointState::Prepared),
                "{id} gained an Applied receipt across the interruption"
            );
            let retry = lease
                .mutate(
                    id,
                    &fingerprint,
                    &relative,
                    effect,
                    &parent,
                    OsStr::new(&relative),
                    |_| Ok(after.map(<[u8]>::to_vec)),
                )
                .unwrap_err();
            ensure!(
                retry.to_string().contains("uncertain"),
                "{id} replayed an interrupted effect: {retry:#}"
            );
            ensure!(
                std::fs::read(&path).ok().as_deref() == after,
                "{id} changed after an uncertain exact retry"
            );
            lease.prune(id, true)?;
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_private_target_mode_and_cleans_stage() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir()?;
        let project = temporary.path().join("project");
        std::fs::create_dir(&project)?;
        let target = project.join("secret.txt");
        std::fs::write(&target, b"before")?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable)?;
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let store = CheckpointStore::new(private.path(), root)?;
        let result = store.lease()?.mutate(
            "private-replace",
            &hash(b"private-replace"),
            "secret.txt",
            FileEffect::Edit,
            &parent,
            OsStr::new("secret.txt"),
            |_| Ok(Some(b"after".to_vec())),
        )?;
        ensure!(
            result.state == CheckpointState::Applied,
            "replacement did not settle"
        );
        ensure!(
            std::fs::read(&target)? == b"after",
            "replacement bytes differ"
        );
        ensure!(
            std::fs::metadata(&target)?.permissions().mode() & 0o777 == 0o600,
            "replacement widened target permissions"
        );
        ensure!(
            std::fs::read_dir(&project)?.count() == 1,
            "staged private directory was not cleaned"
        );
        Ok(())
    }

    #[test]
    fn prepared_capacity_reserves_largest_applied_identity() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let project = temporary.path().join("project");
        std::fs::create_dir(&project)?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let store = CheckpointStore::new(private.path(), root)?;
        let lease = store.lease()?;
        lease.prepare(
            "reserve",
            &hash(b"reserve"),
            "note.txt",
            FileEffect::Write,
            None,
            Some(b"after"),
            None,
            None,
            None,
            None,
        )?;
        let prepared = lease.read("reserve")?.context("prepared receipt missing")?;
        let reserved_size = serde_json::to_vec(&prepared)?.len();
        let physical_bytes = std::fs::read_dir(store.directory.path())?
            .map(|entry| {
                entry
                    .and_then(|entry| entry.metadata())
                    .map(|metadata| metadata.len())
            })
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .sum::<u64>();
        ensure!(
            lease.occupied_bytes()?
                >= physical_bytes
                    + prepared
                        .after
                        .as_ref()
                        .map_or(0, |bytes| bytes.len() as u64),
            "prepared receipt did not reserve a retained stage payload"
        );
        let mut applied = prepared.clone();
        applied.state = CheckpointState::Applied;
        applied.capacity_reservation = None;
        applied.post_identity = Some([u8::MAX; 24]);
        ensure!(
            serde_json::to_vec(&applied)?.len() <= reserved_size,
            "settled receipt can exceed prepared capacity reservation"
        );
        Ok(())
    }

    #[test]
    fn explicit_uncertain_discard_removes_only_its_checked_private_stage() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let project = temporary.path().join("project");
        std::fs::create_dir(&project)?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let parent = Directory::open(&project, Privacy::Inherited, NameRetention::Movable)?;
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let store = CheckpointStore::new(private.path(), root)?;
        let lease = store.lease()?;
        let stage_name = format!(".kuru-edit-{}", hash(b"interrupted"));
        let stage = parent.create_private_directory(OsStr::new(&stage_name))?;
        stage
            .create_new(OsStr::new("payload"))?
            .write_all(b"unpublished")?;
        let template_name = format!(".kuru-edit-template-{}", hash(b"interrupted"));
        let template = parent.create_new(OsStr::new(&template_name))?;
        lease.prepare(
            "interrupted",
            &hash(b"interrupted"),
            "note.txt",
            FileEffect::Write,
            None,
            Some(b"unpublished"),
            None,
            Some(parent.identity().to_bytes()),
            Some(stage.identity().to_bytes()),
            Some(regular_file_info(&template)?.identity.to_bytes()),
        )?;
        ensure!(
            lease.prune("interrupted", false).is_err(),
            "ordinary pruning removed a Prepared recovery record"
        );
        ensure!(
            lease.inspect("interrupted")?.is_some(),
            "ordinary pruning erased an unresolved receipt"
        );
        drop(template);
        drop(stage);
        let saved_stage = project.join(".kuru-edit-saved");
        std::fs::rename(project.join(&stage_name), &saved_stage)?;
        let replacement = parent.create_private_directory(OsStr::new(&stage_name))?;
        let error = lease.prune("interrupted", true).unwrap_err();
        ensure!(
            error.to_string().contains("stage identity changed"),
            "discard accepted a substituted private stage: {error:#}"
        );
        ensure!(
            lease.inspect("interrupted")?.is_some(),
            "substitution erased receipt"
        );
        replacement.remove_tree()?;
        std::fs::rename(&saved_stage, project.join(&stage_name))?;
        ensure!(
            lease.prune("interrupted", true)?,
            "selected receipt was not discarded"
        );
        ensure!(
            lease.inspect("interrupted")?.is_none(),
            "discarded receipt remains"
        );
        ensure!(
            !project.join(stage_name).exists() && !project.join(template_name).exists(),
            "discard left its staged payload or template"
        );
        lease.mutate(
            "interrupted",
            &hash(b"interrupted"),
            "note.txt",
            FileEffect::Write,
            &parent,
            OsStr::new("note.txt"),
            |_| Ok(Some(b"fresh".to_vec())),
        )?;
        ensure!(
            std::fs::read(project.join("note.txt"))? == b"fresh",
            "fresh write after explicit discard was blocked by stale stage"
        );
        Ok(())
    }

    fn hunk(before: &str, old: &str, after: &str, replacement: &str) -> EditHunk {
        EditHunk {
            before: before.into(),
            old: old.into(),
            after: after.into(),
            replacement: replacement.into(),
        }
    }

    #[test]
    fn two_ordered_hunks_apply_against_one_immutable_source() {
        let source = b"alpha\none\nbeta\ntwo\ngamma\n";
        let edits = [
            hunk("alpha\n", "one", "\n", "un"),
            hunk("beta\n", "two", "\n", "deux"),
        ];
        assert_eq!(
            apply_hunks(source, &edits).unwrap(),
            b"alpha\nun\nbeta\ndeux\ngamma\n"
        );
        let stale = [edits[0].clone(), hunk("beta\n", "three", "\n", "trois")];
        assert!(apply_hunks(source, &stale).is_err());
        assert_eq!(source, b"alpha\none\nbeta\ntwo\ngamma\n");
    }

    #[test]
    fn ambiguous_overlapping_and_binary_source_are_rejected() {
        assert!(apply_hunks(b"old old", &[hunk("", "old", "", "new")]).is_err());
        assert!(
            apply_hunks(
                b"abcde",
                &[hunk("a", "b", "c", "B"), hunk("c", "d", "e", "D")]
            )
            .is_err()
        );
        assert!(apply_hunks(b"\xff", &[hunk("", "x", "", "y")]).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn checkpoint_root_accepts_an_existing_non_utf8_project_path() -> Result<()> {
        use std::os::unix::ffi::OsStringExt;
        let temporary = tempfile::tempdir()?;
        let name = std::ffi::OsString::from_vec(b"project-\xff".to_vec());
        let project = temporary.path().join(name);
        std::fs::create_dir(&project)?;
        let root = Arc::new(Directory::open(
            &project,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        let private_parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let private = private_parent.create_private_directory(OsStr::new("private"))?;
        let store = CheckpointStore::new(private.path(), root)?;
        assert!(store.list(1)?.is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn project_digest_uses_native_path_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/project-\xff".to_vec()));
        assert_eq!(
            project_digest(&path),
            hash(path.as_os_str().as_encoded_bytes())
        );
    }
}
