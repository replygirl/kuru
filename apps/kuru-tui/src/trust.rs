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
    Directory, NameRetention, Privacy, Publication, PublicationPhase, regular_file_info,
    seal_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STORE_SCHEMA: u16 = 1;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_CLAIMS: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApprovalState {
    Absent,
    Matching,
    Stale,
    Invalid,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalMethod {
    Command,
    InteractiveTui,
}

#[derive(Debug, Serialize, Deserialize)]
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
            store_schema: STORE_SCHEMA,
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
        self.store_schema == STORE_SCHEMA
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
            Ok(Some(record)) if record.matches(self.root, manifest) => ApprovalState::Matching,
            Ok(Some(record)) if record.structurally_valid() => ApprovalState::Stale,
            Ok(Some(_)) | Err(_) => ApprovalState::Invalid,
        }
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
        let record = ApprovalRecord::current(self.root, manifest, method)?;
        ensure!(
            record.structurally_valid(),
            "workspace authority manifest exceeds approval-record limits"
        );
        let bytes = serde_json::to_vec(&record)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "workspace approval record exceeds its size limit"
        );
        publish(&directory, &record_name(self.root), &bytes)
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

    fn read_record(&self) -> Result<Option<ApprovalRecord>> {
        let Some(directory) = self.open_store()? else {
            return Ok(None);
        };
        let name = record_name(self.root);
        let file = match directory.read(&name) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            regular_file_info(&file)?.len <= MAX_RECORD_BYTES as u64,
            "approval record exceeds its size limit"
        );
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "approval record exceeds its size limit"
        );
        // Keep the opened object authoritative across the read. An atomic
        // replacement after open must not let detached stale bytes grant.
        directory.verify(&name, &file)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
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
        replace_record(&directory, &name, &vec![b'x'; MAX_RECORD_BYTES + 1]);
        assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);

        #[cfg(unix)]
        {
            replace_record(&directory, &name, b"{}");
            std::fs::hard_link(
                directory.path().join(&name),
                directory.path().join("linked-copy"),
            )
            .unwrap();
            assert_eq!(store.inspect(&manifest), ApprovalState::Invalid);
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
