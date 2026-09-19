//! Private tool grants are operational authority, separate from trust and Dolt.

use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, ensure};
use kuru_connectors::permissions::{
    GrantScope, PermissionBinding, PermissionGrantStore, PersistentGrant,
};
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::trust::{ensure_outside_root, exists, lock, private_child, publish};

const STORE_SCHEMA: u16 = 1;
const MAX_GRANTS: usize = 128;
// Shared checked publication also bounds uncertain-write reconciliation to 64 KiB.
const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantRecord {
    schema: u16,
    binding: PermissionBinding,
    scopes: Vec<GrantScope>,
}

pub(crate) struct GrantStore {
    data: PathBuf,
    root: Arc<Directory>,
    binding: PermissionBinding,
    name: String,
    lock_name: String,
}

impl GrantStore {
    pub(crate) fn new(
        data: &Path,
        root: Arc<Directory>,
        binding: PermissionBinding,
    ) -> Result<Self> {
        binding.validate_root(&root)?;
        ensure_outside_root(data, &root)?;
        let digest = Sha256::digest(serde_json::to_vec(&binding)?);
        let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            data: data.to_path_buf(),
            root,
            binding,
            name: format!("{key}.json"),
            lock_name: format!("{key}.lock"),
        })
    }

    fn check_binding(&self, binding: &PermissionBinding) -> Result<()> {
        binding.validate_root(&self.root)?;
        ensure!(
            binding == &self.binding,
            "tool grant authority context changed"
        );
        Ok(())
    }

    fn open_store(&self) -> Result<Option<Directory>> {
        if !exists(&self.data)? {
            return Ok(None);
        }
        let data = Directory::open(&self.data, Privacy::OwnerOnly, NameRetention::Pinned)?;
        ensure!(
            !data.is_within(&self.root)?,
            "tool grants must be outside the workspace"
        );
        let permissions = data.path().join("permissions");
        if !exists(&permissions)? {
            return Ok(None);
        }
        let permissions = Directory::open(&permissions, Privacy::OwnerOnly, NameRetention::Pinned)?;
        let grants = permissions.path().join("grants");
        if !exists(&grants)? {
            return Ok(None);
        }
        Ok(Some(Directory::open(
            &grants,
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )?))
    }

    fn create_store(&self) -> Result<Directory> {
        ensure_outside_root(&self.data, &self.root)?;
        let data = Directory::ensure_private(&self.data)?;
        ensure!(
            !data.is_within(&self.root)?,
            "tool grants must be outside the workspace"
        );
        let permissions = private_child(&data, OsStr::new("permissions"))?;
        private_child(&permissions, OsStr::new("grants"))
    }

    /// Caller retains the context's lock across this read and any replacement.
    fn read_locked(&self, directory: &Directory) -> Result<Vec<GrantScope>> {
        let name = OsStr::new(&self.name);
        let file = match directory.read(name) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            regular_file_info(&file)?.len <= MAX_RECORD_BYTES as u64,
            "tool grant record is too large"
        );
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "tool grant record is too large"
        );
        directory.verify(name, &file)?;
        let record: GrantRecord = serde_json::from_slice(&bytes)?;
        ensure!(
            record.schema == STORE_SCHEMA,
            "unsupported tool grant record"
        );
        self.check_binding(&record.binding)?;
        ensure!(record.scopes.len() <= MAX_GRANTS, "too many tool grants");
        for (index, scope) in record.scopes.iter().enumerate() {
            scope.validate()?;
            ensure!(
                !record.scopes[..index].contains(scope),
                "duplicate tool grant scope"
            );
        }
        directory.revalidate()?;
        Ok(record.scopes)
    }

    fn write_locked(&self, directory: &Directory, scopes: Vec<GrantScope>) -> Result<()> {
        self.check_binding(&self.binding)?;
        ensure!(scopes.len() <= MAX_GRANTS, "too many tool grants");
        let bytes = serde_json::to_vec(&GrantRecord {
            schema: STORE_SCHEMA,
            binding: self.binding.clone(),
            scopes,
        })?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "tool grant record is too large"
        );
        publish(directory, OsStr::new(&self.name), &bytes)?;
        self.root.revalidate()?;
        Ok(())
    }
}

impl PermissionGrantStore for GrantStore {
    fn load(&self, binding: &PermissionBinding) -> Result<Vec<PersistentGrant>> {
        self.check_binding(binding)?;
        let Some(directory) = self.open_store()? else {
            return Ok(Vec::new());
        };
        let _lock = lock(&directory, OsStr::new(&self.lock_name))?;
        Ok(self
            .read_locked(&directory)?
            .into_iter()
            .map(|scope| PersistentGrant {
                binding: self.binding.clone(),
                scope,
            })
            .collect())
    }

    fn add(&self, grant: &PersistentGrant) -> Result<()> {
        self.check_binding(&grant.binding)?;
        grant.scope.validate()?;
        let directory = self.create_store()?;
        let _lock = lock(&directory, OsStr::new(&self.lock_name))?;
        let mut scopes = self.read_locked(&directory)?;
        if !scopes.contains(&grant.scope) {
            scopes.push(grant.scope.clone());
            self.write_locked(&directory, scopes)?;
        }
        Ok(())
    }

    fn revoke(&self, grant: &PersistentGrant) -> Result<bool> {
        self.check_binding(&grant.binding)?;
        grant.scope.validate()?;
        let Some(directory) = self.open_store()? else {
            return Ok(false);
        };
        let _lock = lock(&directory, OsStr::new(&self.lock_name))?;
        let mut scopes = self.read_locked(&directory)?;
        let before = scopes.len();
        scopes.retain(|stored| stored != &grant.scope);
        if before == scopes.len() {
            return Ok(false);
        }
        // Keep an empty checked record so revocation uses the same durable,
        // reconciled publication as insertion, including on Windows.
        self.write_locked(&directory, scopes)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{
        Config, ConfigSnapshot, InvocationOverrides, NativeTool, PermissionSelector,
        ProjectRelativeTarget,
    };

    struct Fixture {
        _temporary: tempfile::TempDir,
        data: PathBuf,
        root: Arc<Directory>,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let project = temporary.path().join("project");
            std::fs::create_dir(&project).unwrap();
            let root = Arc::new(
                Directory::open(&project, Privacy::Inherited, NameRetention::Pinned).unwrap(),
            );
            Self {
                data: temporary.path().join("data"),
                root,
                _temporary: temporary,
            }
        }

        fn store(&self, config: &Config) -> GrantStore {
            let snapshot =
                ConfigSnapshot::parse(None, self.root.path(), None, InvocationOverrides::default())
                    .unwrap();
            let binding =
                PermissionBinding::checked(&self.root, snapshot.manifest().full_digest(), config)
                    .unwrap();
            GrantStore::new(&self.data, self.root.clone(), binding).unwrap()
        }
    }

    fn file_grant(store: &GrantStore, path: &str) -> PersistentGrant {
        PersistentGrant {
            binding: store.binding.clone(),
            scope: GrantScope::ExactFile {
                selector: PermissionSelector::native(NativeTool::FileWrite),
                target: ProjectRelativeTarget::parse(path).unwrap(),
            },
        }
    }

    #[test]
    fn absent_inspection_and_revocation_create_no_state() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        assert!(store.load(&store.binding).unwrap().is_empty());
        assert!(!store.revoke(&file_grant(&store, "first.txt")).unwrap());
        assert!(!fixture.data.exists());
    }

    #[test]
    fn independent_handles_preserve_literal_scopes_restart_and_revoke() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let first = fixture.store(&Config::default());
        let second = fixture.store(&Config::default());
        let literal = file_grant(&first, "src/[notes]*?.txt");
        let ordinary = file_grant(&second, "src/other.txt");
        first.add(&literal).unwrap();
        second.add(&ordinary).unwrap();
        second.add(&literal).unwrap();
        drop(first);
        let restarted = fixture.store(&Config::default());
        let restored = restarted.load(&restarted.binding).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.contains(&literal));
        assert!(restored.contains(&ordinary));
        assert!(restarted.revoke(&literal).unwrap());
        assert!(!restarted.revoke(&literal).unwrap());
        assert_eq!(
            second.load(&second.binding).unwrap(),
            vec![ordinary.clone()]
        );
        assert!(second.revoke(&ordinary).unwrap());
        assert!(restarted.load(&restarted.binding).unwrap().is_empty());
    }

    #[test]
    fn changed_authority_cannot_read_or_modify_prior_grants() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let first = fixture.store(&Config::default());
        let grant = file_grant(&first, "notes.txt");
        first.add(&grant).unwrap();
        let changed = fixture.store(&Config {
            allow_shell: true,
            ..Config::default()
        });
        assert!(changed.load(&changed.binding).unwrap().is_empty());
        assert!(changed.load(&first.binding).is_err());
        assert!(changed.add(&grant).is_err());
        assert!(changed.revoke(&grant).is_err());
        assert_eq!(first.load(&first.binding).unwrap(), vec![grant]);
        let other = Fixture::new();
        assert!(GrantStore::new(&other.data, other.root.clone(), first.binding).is_err());
        assert!(!other.data.exists());
    }

    #[test]
    fn corrupt_or_oversized_private_record_is_an_error_not_an_empty_grant_set() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        let grant = file_grant(&store, "notes.txt");
        store.add(&grant).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        publish(&directory, OsStr::new(&store.name), b"not json").unwrap();
        assert!(store.load(&store.binding).is_err());
        assert!(store.add(&grant).is_err());
        assert!(store.revoke(&grant).is_err());
        publish(
            &directory,
            OsStr::new(&store.name),
            &vec![b' '; MAX_RECORD_BYTES + 1],
        )
        .unwrap();
        assert!(store.load(&store.binding).is_err());
    }

    #[test]
    fn invalid_record_scope_and_schema_are_rejected() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        let grant = file_grant(&store, "notes.txt");
        store.add(&grant).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        for payload in [
            serde_json::json!({"schema": 2, "binding": store.binding, "scopes": []}),
            serde_json::json!({"schema": 1, "binding": store.binding, "scopes": [grant.scope, grant.scope]}),
            serde_json::json!({"schema": 1, "binding": store.binding, "scopes": [{"kind": "whole_tool", "selector": {"kind": "native", "name": "file_write"}}]}),
        ] {
            publish(
                &directory,
                OsStr::new(&store.name),
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
            assert!(store.load(&store.binding).is_err());
        }
    }

    #[test]
    fn grant_capacity_failure_preserves_the_previous_record() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        let directory = store.create_store().unwrap();
        let scopes: Vec<_> = (0..MAX_GRANTS)
            .map(|index| file_grant(&store, &format!("notes/{index}.txt")).scope)
            .collect();
        store.write_locked(&directory, scopes).unwrap();
        assert!(store.add(&file_grant(&store, "one-too-many.txt")).is_err());
        assert_eq!(store.load(&store.binding).unwrap().len(), MAX_GRANTS);
    }

    #[test]
    fn held_store_lock_and_in_workspace_storage_fail_closed() {
        let _gate = crate::spawn_gate::locking();
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        let grant = file_grant(&store, "notes.txt");
        store.add(&grant).unwrap();
        let directory = store.open_store().unwrap().unwrap();
        let held = lock(&directory, OsStr::new(&store.lock_name)).unwrap();
        assert!(store.load(&store.binding).is_err());
        assert!(store.add(&grant).is_err());
        assert!(store.revoke(&grant).is_err());
        drop(held);
        assert_eq!(store.load(&store.binding).unwrap(), vec![grant]);
        assert!(
            GrantStore::new(
                &fixture.root.path().join("state"),
                fixture.root.clone(),
                store.binding
            )
            .is_err()
        );
        assert!(!fixture.root.path().join("state").exists());
    }

    #[cfg(unix)]
    #[test]
    fn replaced_store_path_cannot_reuse_held_authority() {
        let _gate = crate::spawn_gate::locking();
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let store = fixture.store(&Config::default());
        let grant = file_grant(&store, "notes.txt");
        store.add(&grant).unwrap();
        let path = fixture.data.join("permissions/grants");
        let moved = fixture.data.join("permissions/old-grants");
        std::fs::rename(&path, &moved).unwrap();
        symlink(&moved, &path).unwrap();
        assert!(store.load(&store.binding).is_err());
        assert!(store.add(&grant).is_err());
        assert!(store.revoke(&grant).is_err());
    }
}
