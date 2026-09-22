//! Workspace trust policy and checked local approval state.
//!
//! Trust authorizes automatic ancestor configuration. It is not a filesystem
//! sandbox and never widens the retained workspace capability.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{ErrorKind, Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::AuthorityManifest;
use kuru_platform::fs::{
    Directory, FileIdentity, NameRetention, Privacy, Publication, PublicationPhase,
    regular_file_info, seal_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const LEGACY_STORE_SCHEMA: u16 = 1;
const STORE_SCHEMA: u16 = 2;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_CLAIMS: usize = 512;
const MAX_NESTED_APPROVALS: usize = 64;
const MAX_NESTED_SOURCES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApprovalState {
    Absent,
    Matching,
    Stale,
    Invalid,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalMethod {
    Command,
    InteractiveTui,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRecord {
    store_schema: u16,
    manifest_schema: u16,
    root_digest: String,
    root_identity: String,
    manifest_digest: String,
    claim_digests: Vec<String>,
    approved_at_unix_seconds: u64,
    method: ApprovalMethod,
}

impl ApprovalRecord {
    fn current(
        root: &Directory,
        manifest: &AuthorityManifest,
        method: ApprovalMethod,
    ) -> Result<Self> {
        let approved_at_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock precedes the Unix epoch")?
            .as_secs();
        Ok(Self {
            store_schema: LEGACY_STORE_SCHEMA,
            manifest_schema: manifest.schema_version(),
            root_digest: root_digest(root),
            root_identity: hex(&root.identity().to_bytes()),
            manifest_digest: manifest.full_digest().to_string(),
            claim_digests: claim_digests(manifest),
            approved_at_unix_seconds,
            method,
        })
    }

    fn structurally_valid(&self) -> bool {
        self.store_schema == LEGACY_STORE_SCHEMA
            && self.root_digest.len() == 64
            && is_lower_hex(&self.root_digest)
            && self.root_identity.len() == 48
            && is_lower_hex(&self.root_identity)
            && self.manifest_digest.len() == 64
            && is_lower_hex(&self.manifest_digest)
            && self.claim_digests.len() <= MAX_CLAIMS
            && self
                .claim_digests
                .iter()
                .all(|digest| digest.len() == 64 && is_lower_hex(digest))
            && self.claim_digests.windows(2).all(|pair| pair[0] <= pair[1])
    }

    fn matches(&self, root: &Directory, manifest: &AuthorityManifest) -> bool {
        self.structurally_valid()
            && self.manifest_schema == manifest.schema_version()
            && self.root_digest == root_digest(root)
            && self.root_identity == hex(&root.identity().to_bytes())
            && self.manifest_digest == manifest.full_digest().to_string()
            && self.claim_digests == claim_digests(manifest)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NestedApproval {
    /// SHA-256 digests of the canonical, active nested source paths.
    source_paths: Vec<String>,
    /// The complete extended authority manifest, not just the added claims.
    approval: ApprovalRecord,
}

impl NestedApproval {
    fn structurally_valid(&self) -> bool {
        !self.source_paths.is_empty()
            && self.source_paths.len() <= MAX_NESTED_SOURCES
            && self
                .source_paths
                .iter()
                .all(|digest| digest.len() == 64 && is_lower_hex(digest))
            && self.source_paths.windows(2).all(|pair| pair[0] < pair[1])
            && self.approval.structurally_valid()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRecordV2 {
    store_schema: u16,
    generation: String,
    base: ApprovalRecord,
    nested: Vec<NestedApproval>,
}

impl ApprovalRecordV2 {
    fn structurally_valid(&self) -> bool {
        self.store_schema == STORE_SCHEMA
            && uuid::Uuid::parse_str(&self.generation)
                .is_ok_and(|value| value.get_version_num() == 4)
            && self.base.structurally_valid()
            && self.nested.len() <= MAX_NESTED_APPROVALS
            && self.nested.iter().all(NestedApproval::structurally_valid)
            && self
                .nested
                .windows(2)
                .all(|pair| pair[0].source_paths < pair[1].source_paths)
            && self.nested.iter().all(|entry| {
                entry.approval.root_digest == self.base.root_digest
                    && entry.approval.root_identity == self.base.root_identity
                    && entry.approval.manifest_schema == self.base.manifest_schema
            })
    }
}

#[derive(Clone, Debug)]
enum StoredRecord {
    Legacy(ApprovalRecord),
    Current(ApprovalRecordV2),
    Invalid,
}

impl StoredRecord {
    fn parse(bytes: &[u8]) -> Self {
        // Parse directly into strict structs: a generic JSON Value would erase
        // duplicate keys before the record shape can reject them.
        if let Ok(record) = serde_json::from_slice::<ApprovalRecordV2>(bytes) {
            Self::Current(record)
        } else if let Ok(record) = serde_json::from_slice::<ApprovalRecord>(bytes) {
            Self::Legacy(record)
        } else {
            Self::Invalid
        }
    }

    fn structurally_valid(&self) -> bool {
        match self {
            Self::Legacy(record) => record.structurally_valid(),
            Self::Current(record) => record.structurally_valid(),
            Self::Invalid => false,
        }
    }

    fn base(&self) -> Option<&ApprovalRecord> {
        match self {
            Self::Legacy(record) => Some(record),
            Self::Current(record) => Some(&record.base),
            Self::Invalid => None,
        }
    }
}

struct CheckedRecord {
    record: StoredRecord,
    identity: FileIdentity,
}

/// Exact pre-review state; publication rejects a revoked or replaced base.
#[derive(Clone, Debug)]
pub(crate) enum ReviewGeneration {
    Absent,
    Legacy(FileIdentity),
    Current(String),
}

pub(crate) struct ApprovalStore<'a> {
    data: &'a Path,
    root: &'a Directory,
}

impl<'a> ApprovalStore<'a> {
    pub(crate) fn new(data: &'a Path, root: &'a Directory) -> Self {
        Self { data, root }
    }

    /// Inspect existing approval state without creating any directory, file, or lock.
    pub(crate) fn inspect(&self, manifest: &AuthorityManifest) -> ApprovalState {
        match self.read_record() {
            Ok(None) => ApprovalState::Absent,
            Ok(Some(checked))
                if checked.record.structurally_valid()
                    && checked
                        .record
                        .base()
                        .is_some_and(|base| base.matches(self.root, manifest)) =>
            {
                ApprovalState::Matching
            }
            Ok(Some(checked)) if checked.record.structurally_valid() => ApprovalState::Stale,
            Ok(Some(_)) | Err(_) => ApprovalState::Invalid,
        }
    }

    /// Inspect a path-qualified complete approval and retain its publication generation.
    pub(crate) fn inspect_nested(
        &self,
        base: &AuthorityManifest,
        extended: &AuthorityManifest,
        source_paths: &[[u8; 32]],
    ) -> (ApprovalState, Option<ReviewGeneration>) {
        if !valid_source_paths(source_paths) {
            return (ApprovalState::Invalid, None);
        }
        let source_paths = hex_source_paths(source_paths);
        match self.read_record() {
            Ok(None) => (ApprovalState::Absent, Some(ReviewGeneration::Absent)),
            Ok(Some(checked)) if checked.record.structurally_valid() => {
                let generation = match &checked.record {
                    StoredRecord::Legacy(_) => ReviewGeneration::Legacy(checked.identity),
                    StoredRecord::Current(record) => {
                        ReviewGeneration::Current(record.generation.clone())
                    }
                    StoredRecord::Invalid => unreachable!("guarded by structural validity"),
                };
                if !checked
                    .record
                    .base()
                    .is_some_and(|record| record.matches(self.root, base))
                {
                    return (ApprovalState::Stale, Some(generation));
                }
                let matching = match &checked.record {
                    StoredRecord::Current(record) => record.nested.iter().any(|entry| {
                        entry.source_paths == source_paths
                            && entry.approval.matches(self.root, extended)
                    }),
                    StoredRecord::Legacy(_) | StoredRecord::Invalid => false,
                };
                (
                    if matching {
                        ApprovalState::Matching
                    } else {
                        ApprovalState::Absent
                    },
                    Some(generation),
                )
            }
            Ok(Some(_)) | Err(_) => (ApprovalState::Invalid, None),
        }
    }

    pub(crate) fn generation_is_current(&self, reviewed: &ReviewGeneration) -> Result<bool> {
        Ok(generation_matches(reviewed, self.read_record()?.as_ref()))
    }

    /// Persist one complete current manifest after an explicit user action.
    pub(crate) fn approve_command(&self, manifest: &AuthorityManifest) -> Result<()> {
        self.approve(manifest, ApprovalMethod::Command)
    }

    pub(crate) fn approve_tui(&self, manifest: &AuthorityManifest) -> Result<()> {
        self.approve(manifest, ApprovalMethod::InteractiveTui)
    }

    fn approve(&self, manifest: &AuthorityManifest, method: ApprovalMethod) -> Result<()> {
        ensure_outside_root(self.data, self.root)?;
        let directory = self
            .create_store()
            .map_err(|_| anyhow::anyhow!("workspace approval storage is unavailable or unsafe"))?;
        let _lock = lock(&directory, &lock_name(self.root))
            .map_err(|_| anyhow::anyhow!("workspace approval storage is busy or unsafe"))?;
        let previous = self.read_record()?;
        let base = ApprovalRecord::current(self.root, manifest, method)?;
        let nested = match previous {
            Some(CheckedRecord {
                record: StoredRecord::Current(current),
                ..
            }) if current.structurally_valid() && current.base.matches(self.root, manifest) => {
                current.nested
            }
            _ => Vec::new(),
        };
        self.publish_record(
            &directory,
            ApprovalRecordV2 {
                store_schema: STORE_SCHEMA,
                generation: uuid::Uuid::new_v4().to_string(),
                base,
                nested,
            },
        )
    }

    pub(crate) fn approve_nested(
        &self,
        base: &AuthorityManifest,
        extended: &AuthorityManifest,
        source_paths: &[[u8; 32]],
        reviewed: &ReviewGeneration,
        method: ApprovalMethod,
    ) -> Result<()> {
        ensure!(
            valid_source_paths(source_paths),
            "invalid nested instruction source set"
        );
        let source_paths = hex_source_paths(source_paths);
        ensure_outside_root(self.data, self.root)?;
        let directory = self
            .create_store()
            .map_err(|_| anyhow::anyhow!("workspace approval storage is unavailable or unsafe"))?;
        let _lock = lock(&directory, &lock_name(self.root))
            .map_err(|_| anyhow::anyhow!("workspace approval storage is busy or unsafe"))?;
        let current = self.read_record()?;
        ensure!(
            generation_matches(reviewed, current.as_ref()),
            "workspace approval changed during review; review current authority again"
        );
        let mut record = match current {
            Some(CheckedRecord {
                record: StoredRecord::Current(record),
                ..
            }) if record.structurally_valid() && record.base.matches(self.root, base) => record,
            Some(CheckedRecord {
                record: StoredRecord::Current(record),
                ..
            }) if record.structurally_valid() => ApprovalRecordV2 {
                store_schema: STORE_SCHEMA,
                generation: String::new(),
                base: ApprovalRecord::current(self.root, base, method)?,
                nested: Vec::new(),
            },
            Some(CheckedRecord {
                record: StoredRecord::Legacy(record),
                ..
            }) if record.matches(self.root, base) => ApprovalRecordV2 {
                store_schema: STORE_SCHEMA,
                generation: String::new(),
                base: record,
                nested: Vec::new(),
            },
            Some(CheckedRecord {
                record: StoredRecord::Legacy(_),
                ..
            }) => ApprovalRecordV2 {
                store_schema: STORE_SCHEMA,
                generation: String::new(),
                base: ApprovalRecord::current(self.root, base, method)?,
                nested: Vec::new(),
            },
            None if matches!(reviewed, ReviewGeneration::Absent) => ApprovalRecordV2 {
                store_schema: STORE_SCHEMA,
                generation: String::new(),
                base: ApprovalRecord::current(self.root, base, method)?,
                nested: Vec::new(),
            },
            _ => bail!(
                "workspace base approval changed during review; review current authority again"
            ),
        };
        let entry = NestedApproval {
            source_paths,
            approval: ApprovalRecord::current(self.root, extended, method)?,
        };
        match record
            .nested
            .binary_search_by(|existing| existing.source_paths.cmp(&entry.source_paths))
        {
            Ok(index) => record.nested[index] = entry,
            Err(index) => record.nested.insert(index, entry),
        }
        record.generation = uuid::Uuid::new_v4().to_string();
        self.publish_record(&directory, record)
    }

    fn publish_record(&self, directory: &Directory, record: ApprovalRecordV2) -> Result<()> {
        ensure!(
            record.structurally_valid(),
            "workspace authority manifest exceeds approval-record limits"
        );
        let bytes = serde_json::to_vec(&record)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "workspace approval record exceeds its size limit"
        );
        publish(directory, &record_name(self.root), &bytes)
            .map_err(|_| anyhow::anyhow!("workspace approval could not be published safely"))
    }

    /// Remove an existing checked record. An absent record is a pure no-op.
    pub(crate) fn revoke(&self) -> Result<bool> {
        let Some(directory) = self
            .open_store()
            .map_err(|_| anyhow::anyhow!("workspace approval storage is unavailable or unsafe"))?
        else {
            return Ok(false);
        };
        let operations = movable_record_directory(&directory)
            .map_err(|_| anyhow::anyhow!("workspace approval storage is unavailable or unsafe"))?;
        let name = record_name(self.root);
        let record = match operations.read(&name) {
            Ok(record) => record,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(_) => bail!("workspace approval record is unsafe and was not removed"),
        };
        let _lock = lock(&directory, &lock_name(self.root))
            .map_err(|_| anyhow::anyhow!("workspace approval storage is busy or unsafe"))?;
        directory
            .revalidate()
            .map_err(|_| anyhow::anyhow!("workspace approval storage changed before revocation"))?;
        operations
            .verify(&name, &record)
            .map_err(|_| anyhow::anyhow!("workspace approval record changed before revocation"))?;
        let removed = match operations.remove_file(&name, record) {
            Ok(()) => true,
            Err(error) if error.phase == PublicationPhase::Uncertain => {
                match operations.read(&name) {
                    Err(missing) if missing.kind() == ErrorKind::NotFound => true,
                    _ => bail!("workspace approval revocation could not be verified"),
                }
            }
            Err(_) => bail!("workspace approval record could not be removed safely"),
        };
        directory
            .revalidate()
            .map_err(|_| anyhow::anyhow!("workspace approval storage changed during revocation"))?;
        Ok(removed)
    }

    fn read_record(&self) -> Result<Option<CheckedRecord>> {
        let Some(directory) = self.open_store()? else {
            return Ok(None);
        };
        let name = record_name(self.root);
        let file = match directory.read(&name) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let info = regular_file_info(&file)?;
        let mut bytes = Vec::new();
        if info.len <= MAX_RECORD_BYTES as u64 {
            (&file)
                .take((MAX_RECORD_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
        }
        // Keep the opened object authoritative across the read. An atomic
        // replacement after open must not let detached stale bytes grant.
        directory.verify(&name, &file)?;
        Ok(Some(CheckedRecord {
            record: if info.len > MAX_RECORD_BYTES as u64 || bytes.len() > MAX_RECORD_BYTES {
                StoredRecord::Invalid
            } else {
                StoredRecord::parse(&bytes)
            },
            identity: info.identity,
        }))
    }

    fn open_store(&self) -> Result<Option<Directory>> {
        if !exists(self.data)? {
            return Ok(None);
        }
        let data = Directory::open(self.data, Privacy::OwnerOnly, NameRetention::Pinned)?;
        ensure!(
            !data.is_within(self.root)?,
            "approval storage must be outside the workspace"
        );
        let trust = data.path().join("trust");
        if !exists(&trust)? {
            return Ok(None);
        }
        let trust = Directory::open(&trust, Privacy::OwnerOnly, NameRetention::Pinned)?;
        let workspaces = trust.path().join("workspaces");
        if !exists(&workspaces)? {
            return Ok(None);
        }
        Ok(Some(Directory::open(
            &workspaces,
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )?))
    }

    fn create_store(&self) -> Result<Directory> {
        let data = Directory::ensure_private(self.data)?;
        ensure!(
            !data.is_within(self.root)?,
            "approval storage must be outside the workspace"
        );
        let trust = private_child(&data, OsStr::new("trust"))?;
        private_child(&trust, OsStr::new("workspaces"))
    }
}

pub(crate) fn private_child(parent: &Directory, name: &OsStr) -> Result<Directory> {
    match parent.create_private_directory(name) {
        Ok(created) => {
            drop(created);
            Ok(Directory::open(
                &parent.path().join(name),
                Privacy::OwnerOnly,
                NameRetention::Pinned,
            )?)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(Directory::open(
            &parent.path().join(name),
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )?),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn lock(directory: &Directory, name: &OsStr) -> Result<File> {
    let file = directory.lock_file(name)?;
    file.try_lock()?;
    directory.verify(name, &file)?;
    Ok(file)
}

/// Retain the pinned store directory as the authority anchor while using a
/// separately checked movable handle for record replacement and removal.
fn movable_record_directory(directory: &Directory) -> Result<Directory> {
    directory.revalidate()?;
    let operations = Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Movable)?;
    ensure!(
        operations.identity() == directory.identity(),
        "approval storage changed before record operation"
    );
    Ok(operations)
}

pub(crate) fn publish(directory: &Directory, destination: &OsStr, bytes: &[u8]) -> Result<()> {
    let operations = movable_record_directory(directory)?;
    let pending = pending_name(destination);
    remove_pending(&operations, &pending)?;
    let mut file = operations.create_new(&pending)?;
    file.write_all(bytes)?;
    seal_private(&file, false)?;
    let identity = regular_file_info(&file)?.identity;
    match operations.publish_file(
        &operations,
        &pending,
        &file,
        destination,
        Publication::ReplaceRegular,
    ) {
        Ok(()) => {}
        Err(error) if error.phase == PublicationPhase::Uncertain => {
            reconcile_publication(&operations, destination, identity, bytes)?;
        }
        Err(error) => return Err(error.into()),
    }
    directory.revalidate()?;
    Ok(())
}

fn reconcile_publication(
    directory: &Directory,
    destination: &OsStr,
    expected_identity: kuru_platform::fs::FileIdentity,
    expected_bytes: &[u8],
) -> Result<()> {
    let installed = directory.read(destination)?;
    let info = regular_file_info(&installed)?;
    ensure!(
        info.identity == expected_identity && info.len <= MAX_RECORD_BYTES as u64,
        "published approval identity or size could not be reconciled"
    );
    let mut bytes = Vec::new();
    (&installed)
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes == expected_bytes,
        "published approval payload could not be reconciled"
    );
    directory.verify(destination, &installed)?;
    directory.revalidate()?;
    Ok(())
}

fn remove_pending(directory: &Directory, name: &OsStr) -> Result<()> {
    let file = match directory.read(name) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match directory.remove_file(name, file) {
        Ok(()) => Ok(()),
        Err(error) if error.phase == PublicationPhase::Uncertain => match directory.read(name) {
            Err(missing) if missing.kind() == ErrorKind::NotFound => Ok(()),
            _ => bail!("pending approval removal could not be verified"),
        },
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn ensure_outside_root(data: &Path, root: &Directory) -> Result<()> {
    ensure!(
        !data.starts_with(root.path()),
        "approval storage must be outside the workspace"
    );
    if exists(data)? {
        let data = Directory::open(data, Privacy::Inherited, NameRetention::Pinned)?;
        ensure!(
            !data.is_within(root)?,
            "approval storage must be outside the workspace"
        );
    }
    Ok(())
}

fn record_name(root: &Directory) -> OsString {
    OsString::from(format!("{}.json", root_digest(root)))
}

fn lock_name(root: &Directory) -> OsString {
    OsString::from(format!("{}.lock", root_digest(root)))
}

fn pending_name(destination: &OsStr) -> OsString {
    let mut digest = Sha256::new();
    digest.update(destination.as_encoded_bytes());
    OsString::from(format!("pending-{}.json", hex(&digest.finalize())))
}

fn root_digest(root: &Directory) -> String {
    hex(&Sha256::digest(root.path().as_os_str().as_encoded_bytes()))
}

fn claim_digests(manifest: &AuthorityManifest) -> Vec<String> {
    let mut digests: Vec<_> = manifest
        .claims()
        .iter()
        .map(|claim| claim.digest().to_string())
        .collect();
    digests.sort();
    digests
}

fn valid_source_paths(paths: &[[u8; 32]]) -> bool {
    !paths.is_empty()
        && paths.len() <= MAX_NESTED_SOURCES
        && paths.windows(2).all(|pair| pair[0] < pair[1])
}

fn hex_source_paths(paths: &[[u8; 32]]) -> Vec<String> {
    paths.iter().map(|path| hex(path)).collect()
}

fn generation_matches(reviewed: &ReviewGeneration, current: Option<&CheckedRecord>) -> bool {
    match (reviewed, current) {
        (ReviewGeneration::Absent, None) => true,
        (
            ReviewGeneration::Legacy(expected),
            Some(CheckedRecord {
                record: StoredRecord::Legacy(_),
                identity,
            }),
        ) => expected == identity,
        (
            ReviewGeneration::Current(expected),
            Some(CheckedRecord {
                record: StoredRecord::Current(record),
                ..
            }),
        ) => expected == &record.generation,
        _ => false,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{ConfigSnapshot, InvocationOverrides};
    use std::path::PathBuf;

    struct Fixture {
        _temporary: tempfile::TempDir,
        workspace: Directory,
        project: PathBuf,
        data: PathBuf,
    }

    impl Fixture {
        fn new(config: &str) -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let project = temporary.path().join("project");
            std::fs::create_dir(&project).unwrap();
            std::fs::create_dir(project.join(".kuru")).unwrap();
            std::fs::write(project.join(".kuru/config.toml"), config).unwrap();
            let workspace =
                Directory::open(&project, Privacy::Inherited, NameRetention::Pinned).unwrap();
            let data = temporary.path().join("data");
            Self {
                _temporary: temporary,
                workspace,
                project,
                data,
            }
        }

        fn manifest(&self) -> AuthorityManifest {
            ConfigSnapshot::parse(None, &self.project, None, InvocationOverrides::default())
                .unwrap()
                .manifest()
                .clone()
        }
    }

    fn replace_record(directory: &Directory, name: &OsStr, bytes: &[u8]) {
        let operations = movable_record_directory(directory).unwrap();
        let current = operations.read(name).unwrap();
        operations.remove_file(name, current).unwrap();
        let mut replacement = operations.create_new(name).unwrap();
        replacement.write_all(bytes).unwrap();
        seal_private(&replacement, false).unwrap();
    }

    #[test]
    fn absent_inspection_and_revocation_create_nothing() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        assert_eq!(store.inspect(&fixture.manifest()), ApprovalState::Absent);
        assert!(!store.revoke().unwrap());
        assert!(!fixture.data.exists());
    }

    #[test]
    fn approval_matches_only_the_complete_manifest_and_native_root() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let first = fixture.manifest();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&first).unwrap();
        assert_eq!(store.inspect(&first), ApprovalState::Matching);

        std::fs::write(
            fixture.project.join(".kuru/config.toml"),
            "allow_shell = true\nallow_write = true\n",
        )
        .unwrap();
        assert_eq!(store.inspect(&fixture.manifest()), ApprovalState::Stale);
        assert!(store.revoke().unwrap());
        assert_eq!(store.inspect(&first), ApprovalState::Absent);
    }

    #[test]
    fn nested_approval_keeps_base_and_binds_exact_sources_and_manifest() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let base = fixture.manifest();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&base).unwrap();
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "READ\n").unwrap();
        let snapshot =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (nested, _) = snapshot.with_nested_directories(&["src".into()]).unwrap();
        let paths = nested.nested_instruction_source_paths();
        let (state, generation) = store.inspect_nested(&base, nested.manifest(), &paths);
        assert_eq!(state, ApprovalState::Absent);
        store
            .approve_nested(
                &base,
                nested.manifest(),
                &paths,
                &generation.unwrap(),
                ApprovalMethod::InteractiveTui,
            )
            .unwrap();
        assert_eq!(store.inspect(&base), ApprovalState::Matching);
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Matching
        );

        let mut wrong_paths = paths.clone();
        wrong_paths.push([0xff; 32]);
        assert_eq!(
            store
                .inspect_nested(&base, nested.manifest(), &wrong_paths)
                .0,
            ApprovalState::Absent
        );
        std::fs::write(fixture.project.join("src/AGENTS.md"), "CHANGED\n").unwrap();
        let changed =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (changed, _) = changed.with_nested_directories(&["src".into()]).unwrap();
        assert_eq!(
            store
                .inspect_nested(
                    &base,
                    changed.manifest(),
                    &changed.nested_instruction_source_paths()
                )
                .0,
            ApprovalState::Absent
        );

        store.approve_tui(&base).unwrap();
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Matching,
            "identical base reapproval preserves nested grants"
        );
        std::fs::write(
            fixture.project.join(".kuru/config.toml"),
            "allow_shell = true\nallow_write = true\n",
        )
        .unwrap();
        store.approve_command(&fixture.manifest()).unwrap();
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Stale
        );
    }

    #[test]
    fn complete_nested_review_can_replace_a_stale_base_atomically() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        let old_base = fixture.manifest();
        store.approve_command(&old_base).unwrap();
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "local instruction").unwrap();
        std::fs::write(
            fixture.project.join(".kuru/config.toml"),
            "allow_shell = true\nallow_write = true\n",
        )
        .unwrap();
        let current =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (nested, _) = current.with_nested_directories(&["src".into()]).unwrap();
        let paths = nested.nested_instruction_source_paths();
        let (state, reviewed) = store.inspect_nested(current.manifest(), nested.manifest(), &paths);
        assert_eq!(state, ApprovalState::Stale);
        store
            .approve_nested(
                current.manifest(),
                nested.manifest(),
                &paths,
                &reviewed.unwrap(),
                ApprovalMethod::InteractiveTui,
            )
            .unwrap();
        assert_eq!(store.inspect(current.manifest()), ApprovalState::Matching);
        assert_eq!(store.inspect(&old_base), ApprovalState::Stale);
        assert_eq!(
            store
                .inspect_nested(current.manifest(), nested.manifest(), &paths)
                .0,
            ApprovalState::Matching
        );
    }

    #[test]
    fn pending_nested_review_cannot_resurrect_revoked_or_recreated_base() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "READ\n").unwrap();
        let base = fixture.manifest();
        let snapshot =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (nested, _) = snapshot.with_nested_directories(&["src".into()]).unwrap();
        let paths = nested.nested_instruction_source_paths();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&base).unwrap();
        let (_, stale) = store.inspect_nested(&base, nested.manifest(), &paths);
        let stale = stale.unwrap();
        assert!(store.revoke().unwrap());
        assert!(
            store
                .approve_nested(
                    &base,
                    nested.manifest(),
                    &paths,
                    &stale,
                    ApprovalMethod::InteractiveTui
                )
                .is_err()
        );
        store.approve_command(&base).unwrap();
        assert!(
            store
                .approve_nested(
                    &base,
                    nested.manifest(),
                    &paths,
                    &stale,
                    ApprovalMethod::InteractiveTui
                )
                .is_err()
        );
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Absent
        );
    }

    #[test]
    fn checked_legacy_base_upgrades_without_losing_startup_authority() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let base = fixture.manifest();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&base).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        let legacy =
            ApprovalRecord::current(&fixture.workspace, &base, ApprovalMethod::Command).unwrap();
        replace_record(
            &directory,
            &record_name(&fixture.workspace),
            &serde_json::to_vec(&legacy).unwrap(),
        );
        assert_eq!(store.inspect(&base), ApprovalState::Matching);
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "READ\n").unwrap();
        let snapshot =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (nested, _) = snapshot.with_nested_directories(&["src".into()]).unwrap();
        let paths = nested.nested_instruction_source_paths();
        let (state, generation) = store.inspect_nested(&base, nested.manifest(), &paths);
        assert_eq!(state, ApprovalState::Absent);
        store
            .approve_nested(
                &base,
                nested.manifest(),
                &paths,
                &generation.unwrap(),
                ApprovalMethod::InteractiveTui,
            )
            .unwrap();
        assert_eq!(store.inspect(&base), ApprovalState::Matching);
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Matching
        );
    }

    #[test]
    fn absent_nested_review_publishes_base_and_old_reader_fails_closed() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "READ\n").unwrap();
        let base = fixture.manifest();
        let snapshot =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (nested, _) = snapshot.with_nested_directories(&["src".into()]).unwrap();
        let paths = nested.nested_instruction_source_paths();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        let (state, generation) = store.inspect_nested(&base, nested.manifest(), &paths);
        assert_eq!(state, ApprovalState::Absent);
        store
            .approve_nested(
                &base,
                nested.manifest(),
                &paths,
                &generation.unwrap(),
                ApprovalMethod::InteractiveTui,
            )
            .unwrap();
        assert_eq!(store.inspect(&base), ApprovalState::Matching);
        assert_eq!(
            store.inspect_nested(&base, nested.manifest(), &paths).0,
            ApprovalState::Matching
        );
        let directory = store.open_store().unwrap().unwrap();
        let name = record_name(&fixture.workspace);
        let mut file = directory.read(&name).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        directory.verify(&name, &file).unwrap();
        assert!(serde_json::from_slice::<ApprovalRecord>(&bytes).is_err());
    }

    #[test]
    fn separate_nested_publication_invalidates_a_pending_generation() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        std::fs::create_dir(fixture.project.join("src")).unwrap();
        std::fs::create_dir(fixture.project.join("tests")).unwrap();
        std::fs::write(fixture.project.join("src/AGENTS.md"), "SRC\n").unwrap();
        std::fs::write(fixture.project.join("tests/AGENTS.md"), "TESTS\n").unwrap();
        let base = fixture.manifest();
        let snapshot =
            ConfigSnapshot::parse(None, &fixture.project, None, InvocationOverrides::default())
                .unwrap();
        let (src, _) = snapshot.with_nested_directories(&["src".into()]).unwrap();
        let (tests, _) = snapshot.with_nested_directories(&["tests".into()]).unwrap();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&base).unwrap();
        let (_, generation) = store.inspect_nested(
            &base,
            tests.manifest(),
            &tests.nested_instruction_source_paths(),
        );
        let generation = generation.unwrap();
        store
            .approve_nested(
                &base,
                src.manifest(),
                &src.nested_instruction_source_paths(),
                &generation,
                ApprovalMethod::InteractiveTui,
            )
            .unwrap();
        assert!(
            store
                .approve_nested(
                    &base,
                    tests.manifest(),
                    &tests.nested_instruction_source_paths(),
                    &generation,
                    ApprovalMethod::InteractiveTui
                )
                .is_err()
        );
        assert_eq!(
            store
                .inspect_nested(
                    &base,
                    src.manifest(),
                    &src.nested_instruction_source_paths()
                )
                .0,
            ApprovalState::Matching
        );
    }

    #[test]
    fn malformed_oversized_and_hard_linked_records_never_match() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let manifest = fixture.manifest();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&manifest).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        let name = record_name(&fixture.workspace);
        replace_record(&directory, &name, b"{\"unknown\":true}");
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
        store.approve_command(&manifest).unwrap();
        assert_eq!(store.inspect(&manifest), ApprovalState::Matching);
        let valid = std::fs::read(directory.path().join(&name)).unwrap();
        let duplicated = String::from_utf8(valid).unwrap().replacen(
            "\"store_schema\":2",
            "\"store_schema\":2,\"store_schema\":2",
            1,
        );
        replace_record(&directory, &name, duplicated.as_bytes());
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
        store.approve_command(&manifest).unwrap();
        replace_record(&directory, &name, &vec![b'x'; MAX_RECORD_BYTES + 1]);
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
        store.approve_command(&manifest).unwrap();
        assert_eq!(store.inspect(&manifest), ApprovalState::Matching);

        #[cfg(unix)]
        {
            replace_record(&directory, &name, b"{}");
            std::fs::hard_link(
                directory.path().join(&name),
                directory.path().join("linked-copy"),
            )
            .unwrap();
            assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
            assert!(store.approve_command(&manifest).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_and_permissive_approval_objects_never_match() {
        let _gate = crate::spawn_gate::locking();
        use std::os::unix::fs::{PermissionsExt, symlink};

        let permissive = Fixture::new("allow_shell = true\n");
        let manifest = permissive.manifest();
        let store = ApprovalStore::new(&permissive.data, &permissive.workspace);
        store.approve_command(&manifest).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        let name = record_name(&permissive.workspace);
        std::fs::set_permissions(
            directory.path().join(&name),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
        assert!(store.approve_command(&manifest).is_err());

        let linked = Fixture::new("allow_shell = true\n");
        let manifest = linked.manifest();
        let store = ApprovalStore::new(&linked.data, &linked.workspace);
        store.approve_command(&manifest).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        let name = record_name(&linked.workspace);
        std::fs::remove_file(directory.path().join(&name)).unwrap();
        let target = linked.project.parent().unwrap().join("outside-record");
        std::fs::write(&target, b"{}").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, directory.path().join(&name)).unwrap();
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);

        let permissive_directory = Fixture::new("allow_shell = true\n");
        let manifest = permissive_directory.manifest();
        let store = ApprovalStore::new(&permissive_directory.data, &permissive_directory.workspace);
        store.approve_command(&manifest).unwrap();
        let path = store.open_store().unwrap().unwrap().path().to_path_buf();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
    }

    #[test]
    fn oversized_claim_sets_and_unreconciled_publications_fail_closed() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new("allow_shell = true\n");
        let manifest = fixture.manifest();
        let store = ApprovalStore::new(&fixture.data, &fixture.workspace);
        store.approve_command(&manifest).unwrap();
        let mut record =
            ApprovalRecord::current(&fixture.workspace, &manifest, ApprovalMethod::Command)
                .unwrap();
        record.claim_digests = vec!["00".repeat(32); MAX_CLAIMS + 1];
        assert!(!record.structurally_valid());

        let base = ApprovalRecord::current(&fixture.workspace, &manifest, ApprovalMethod::Command)
            .unwrap();
        let entry = NestedApproval {
            source_paths: vec!["01".repeat(32)],
            approval: base.clone(),
        };
        let mut current = ApprovalRecordV2 {
            store_schema: STORE_SCHEMA,
            generation: uuid::Uuid::new_v4().to_string(),
            base,
            nested: vec![entry.clone()],
        };
        assert!(current.structurally_valid());
        current.nested = vec![entry; MAX_NESTED_APPROVALS + 1];
        assert!(!current.structurally_valid());
        current.nested.truncate(1);
        current.generation = "reused-content-digest".into();
        assert!(!current.structurally_valid());

        let directory = store.open_store().unwrap().unwrap();
        let expected_name = OsStr::new("expected-candidate");
        let mut expected = directory.create_new(expected_name).unwrap();
        expected.write_all(b"different").unwrap();
        seal_private(&expected, false).unwrap();
        let expected_identity = regular_file_info(&expected).unwrap().identity;
        assert!(
            reconcile_publication(
                &directory,
                &record_name(&fixture.workspace),
                expected_identity,
                b"different"
            )
            .is_err()
        );
    }
}
