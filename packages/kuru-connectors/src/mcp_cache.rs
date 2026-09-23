//! Owner-private, context-bound MCP discovery metadata.
//!
//! These records never establish a live route or permission. `mcp` owns the
//! transition from validated cached metadata to an explicitly stale catalog.

use std::{
    collections::BTreeSet,
    ffi::OsStr,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use kuru_core::ManifestDigest;
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, regular_file_info};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const SCHEMA: u16 = 1;
const MAX_RECORD_BYTES: usize = 512 * 1024;
const MAX_TOOLS: usize = 256;
const MAX_ORIGINAL_NAME_CHARS: usize = 256;
const MAX_DESCRIPTION_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedMcpTool {
    pub(crate) original_name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CatalogRecord {
    schema: u16,
    alias: String,
    project_identity: [u8; 24],
    authority: [u8; 32],
    context: [u8; 32],
    tools: Vec<CachedMcpTool>,
}

/// Application-injected private storage. The retained project handle and
/// authority digest are checked again on every operation.
pub struct McpCatalogStore {
    data: PathBuf,
    root: Arc<Directory>,
    project_identity: [u8; 24],
    authority: [u8; 32],
    directory_name: String,
}

impl McpCatalogStore {
    pub fn new(data: &Path, root: Arc<Directory>, authority: ManifestDigest) -> Result<Self> {
        root.revalidate()?;
        match Directory::open(data, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(existing) => ensure_outside_root(&existing, &root)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("open private MCP catalog data directory");
            }
        }
        let digest = Sha256::digest(root.path().as_os_str().as_encoded_bytes());
        let project = hex(&digest);
        Ok(Self {
            data: data.to_path_buf(),
            project_identity: root.identity().to_bytes(),
            root,
            authority: authority.as_bytes(),
            directory_name: format!("mcp-catalogs-{project}"),
        })
    }

    pub(crate) fn load(
        &self,
        alias: &str,
        context: [u8; 32],
    ) -> Result<Option<Vec<CachedMcpTool>>> {
        self.check_root()?;
        let Some(directory) = self.open_directory()? else {
            return Ok(None);
        };
        let names = names(alias);
        let lock = directory.lock_file(OsStr::new(&names.lock))?;
        lock.try_lock().context("MCP catalog cache is busy")?;
        directory.verify(OsStr::new(&names.lock), &lock)?;
        let file = match directory.read(OsStr::new(&names.record)) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            regular_file_info(&file)?.len <= MAX_RECORD_BYTES as u64,
            "MCP catalog cache record exceeds its byte limit"
        );
        let mut bytes = Vec::new();
        (&file)
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "MCP catalog cache record exceeds its byte limit"
        );
        directory.verify(OsStr::new(&names.record), &file)?;
        let record: CatalogRecord =
            serde_json::from_slice(&bytes).context("MCP catalog cache record is malformed")?;
        self.validate_record(&record, alias, context)?;
        Ok(Some(record.tools))
    }

    pub(crate) fn save(
        &self,
        alias: &str,
        context: [u8; 32],
        tools: Vec<CachedMcpTool>,
    ) -> Result<()> {
        self.check_root()?;
        let record = CatalogRecord {
            schema: SCHEMA,
            alias: alias.to_owned(),
            project_identity: self.project_identity,
            authority: self.authority,
            context,
            tools,
        };
        self.validate_record(&record, alias, context)?;
        let bytes = serde_json::to_vec(&record)?;
        ensure!(
            bytes.len() <= MAX_RECORD_BYTES,
            "MCP catalog cache record exceeds its byte limit"
        );
        let directory = self.create_directory()?;
        let names = names(alias);
        let lock = directory.lock_file(OsStr::new(&names.lock))?;
        lock.try_lock().context("MCP catalog cache is busy")?;
        directory.verify(OsStr::new(&names.lock), &lock)?;
        let temporary = format!("catalog-{}.tmp", uuid::Uuid::new_v4());
        let mut file = directory.create_new(OsStr::new(&temporary))?;
        let publication = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            self.check_root()?;
            directory
                .publish_file(
                    &directory,
                    OsStr::new(&temporary),
                    &file,
                    OsStr::new(&names.record),
                    Publication::ReplaceRegular,
                )
                .context("publish MCP catalog cache record")?;
            directory.verify(OsStr::new(&names.record), &file)?;
            self.check_root()
        })();
        if let Err(error) = publication {
            let _ = directory.remove_file(OsStr::new(&temporary), file);
            return Err(error);
        }
        Ok(())
    }

    fn validate_record(
        &self,
        record: &CatalogRecord,
        alias: &str,
        context: [u8; 32],
    ) -> Result<()> {
        ensure!(
            record.schema == SCHEMA,
            "unsupported MCP catalog cache schema"
        );
        ensure!(record.alias == alias, "MCP catalog cache alias changed");
        ensure!(
            record.project_identity == self.project_identity
                && record.authority == self.authority
                && record.context == context,
            "MCP catalog cache authority context changed"
        );
        ensure!(
            record.tools.len() <= MAX_TOOLS,
            "MCP catalog cache exceeds its tool limit"
        );
        let mut names = BTreeSet::new();
        for tool in &record.tools {
            ensure!(
                !tool.original_name.is_empty()
                    && tool.original_name.chars().count() <= MAX_ORIGINAL_NAME_CHARS
                    && !tool.original_name.chars().any(char::is_control),
                "MCP catalog cache contains an invalid original tool name"
            );
            ensure!(
                names.insert(&tool.original_name),
                "MCP catalog cache contains a duplicate original tool name"
            );
            ensure!(
                tool.description.len() <= MAX_DESCRIPTION_BYTES,
                "MCP catalog cache contains an oversized description"
            );
            ensure!(
                tool.parameters.is_object(),
                "MCP catalog cache contains a non-object input schema"
            );
        }
        Ok(())
    }

    fn check_root(&self) -> Result<()> {
        self.root.revalidate()?;
        ensure!(
            self.root.identity().to_bytes() == self.project_identity,
            "MCP catalog cache project root changed"
        );
        Ok(())
    }

    fn open_directory(&self) -> Result<Option<Directory>> {
        let data = match Directory::open(&self.data, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("open private MCP catalog data directory"),
        };
        ensure_outside_root(&data, &self.root)?;
        match Directory::open(
            &data.path().join(&self.directory_name),
            Privacy::OwnerOnly,
            NameRetention::Movable,
        ) {
            Ok(directory) => Ok(Some(directory)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("open private MCP catalog directory"),
        }
    }

    fn create_directory(&self) -> Result<Directory> {
        let data = Directory::ensure_private(&self.data)
            .context("MCP catalog data directory is not owner-private")?;
        ensure_outside_root(&data, &self.root)?;
        let name = OsStr::new(&self.directory_name);
        match data.create_private_directory(name) {
            Ok(directory) => Ok(directory),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Directory::open(
                &data.path().join(name),
                Privacy::OwnerOnly,
                NameRetention::Movable,
            )
            .context("open private MCP catalog directory"),
            Err(error) => Err(error).context("create private MCP catalog directory"),
        }
    }
}

struct Names {
    record: String,
    lock: String,
}

fn names(alias: &str) -> Names {
    let digest = Sha256::digest(alias.as_bytes());
    let key = hex(&digest);
    Names {
        record: format!("{key}.json"),
        lock: format!("{key}.lock"),
    }
}

pub(crate) fn ensure_outside_root(data: &Directory, root: &Directory) -> Result<()> {
    ensure!(
        !data.is_within(root)? && !root.is_within(data)?,
        "MCP catalog data directory overlaps the workspace"
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{ConfigSnapshot, InvocationOverrides};
    use kuru_platform::fs::{NameRetention, Privacy};

    struct Fixture {
        _project: tempfile::TempDir,
        _private: tempfile::TempDir,
        store: McpCatalogStore,
    }

    fn fixture() -> Fixture {
        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let root = Arc::new(
            Directory::open(project.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let private_parent =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let private_data = private_parent
            .create_private_directory(OsStr::new("data"))
            .unwrap();
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        let store =
            McpCatalogStore::new(private_data.path(), root, snapshot.manifest().full_digest())
                .unwrap();
        Fixture {
            _project: project,
            _private: private,
            store,
        }
    }

    fn tool(name: &str) -> CachedMcpTool {
        CachedMcpTool {
            original_name: name.into(),
            description: "fixture".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }

    fn record_path(fixture: &Fixture, alias: &str) -> PathBuf {
        fixture
            .store
            .data
            .join(&fixture.store.directory_name)
            .join(names(alias).record)
    }

    #[test]
    fn cache_round_trip_is_context_bound_and_partial_files_are_invisible() {
        let fixture = fixture();
        let context = [7; 32];
        fixture
            .store
            .save("remote", context, vec![tool("read")])
            .unwrap();
        assert_eq!(
            fixture.store.load("remote", context).unwrap(),
            Some(vec![tool("read")])
        );
        assert!(fixture.store.load("remote", [8; 32]).is_err());

        let directory = fixture.store.create_directory().unwrap();
        let mut partial = directory
            .create_new(OsStr::new("catalog-interrupted.tmp"))
            .unwrap();
        partial.write_all(b"not a record").unwrap();
        partial.sync_all().unwrap();
        assert_eq!(
            fixture.store.load("missing", context).unwrap(),
            None,
            "unpublished staging bytes cannot become catalog metadata"
        );
    }

    #[test]
    fn malformed_versioned_and_over_limit_records_fail_closed() {
        let fixture = fixture();
        let context = [3; 32];
        fixture
            .store
            .save("remote", context, vec![tool("read")])
            .unwrap();
        std::fs::write(record_path(&fixture, "remote"), b"{").unwrap();
        assert!(fixture.store.load("remote", context).is_err());

        let record = CatalogRecord {
            schema: SCHEMA + 1,
            alias: "remote".into(),
            project_identity: fixture.store.project_identity,
            authority: fixture.store.authority,
            context,
            tools: vec![tool("read")],
        };
        std::fs::write(
            record_path(&fixture, "remote"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(fixture.store.load("remote", context).is_err());

        assert!(
            fixture
                .store
                .save("many", context, vec![tool("read"); MAX_TOOLS + 1])
                .is_err()
        );
        assert!(
            fixture
                .store
                .save("duplicate", context, vec![tool("read"), tool("read")])
                .is_err()
        );
        let mut oversized = tool("read");
        oversized.description = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
        assert!(
            fixture
                .store
                .save("description", context, vec![oversized])
                .is_err()
        );
        let oversized = tool(&"x".repeat(MAX_ORIGINAL_NAME_CHARS + 1));
        assert!(
            fixture
                .store
                .save("name", context, vec![oversized])
                .is_err()
        );
        let mut oversized = tool("schema");
        oversized.parameters =
            serde_json::json!({"type":"object","description":"x".repeat(MAX_RECORD_BYTES)});
        assert!(
            fixture
                .store
                .save("record", context, vec![oversized])
                .is_err()
        );
    }

    #[test]
    fn root_substitution_and_non_object_schemas_are_rejected() {
        let fixture = fixture();
        let context = [4; 32];
        let mut invalid = tool("read");
        invalid.parameters = serde_json::json!([]);
        assert!(
            fixture
                .store
                .save("remote", context, vec![invalid])
                .is_err()
        );

        let replaced = fixture._project.path().with_extension("replaced");
        std::fs::rename(fixture._project.path(), &replaced).unwrap();
        std::fs::create_dir(fixture._project.path()).unwrap();
        assert!(fixture.store.load("remote", context).is_err());
        std::fs::remove_dir(fixture._project.path()).unwrap();
        std::fs::rename(&replaced, fixture._project.path()).unwrap();
        // Keep TempDir cleanup aligned with the restored checked object.
        fixture.store.check_root().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn existing_non_private_data_directory_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let project = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::set_permissions(private.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let root = Arc::new(
            Directory::open(project.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &["/help", "/tools"],
            InvocationOverrides::default(),
        )
        .unwrap();
        assert!(
            McpCatalogStore::new(private.path(), root, snapshot.manifest().full_digest()).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn substituted_record_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = fixture();
        let context = [5; 32];
        fixture
            .store
            .save("remote", context, vec![tool("read")])
            .unwrap();
        let record = record_path(&fixture, "remote");
        let substitute = fixture._private.path().join("substitute.json");
        std::fs::write(&substitute, std::fs::read(&record).unwrap()).unwrap();
        std::fs::remove_file(&record).unwrap();
        symlink(substitute, record).unwrap();
        assert!(fixture.store.load("remote", context).is_err());
    }
}
