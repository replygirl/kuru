//! Pre-migrated store template behind [`MemoryStore::temporary`].
//!
//! A cold fixture open spends most of its time initializing the database,
//! applying every migration and validating the staged and active stores. The
//! template performs that complete cold open once per fingerprint, closes it
//! cleanly, and keeps a private snapshot of the stopped project directory
//! beside the prepared supervisor snapshot:
//!
//! ```text
//! <profile>/kuru-test-templates/<fingerprint>.lock
//! <profile>/kuru-test-templates/<fingerprint>/manifest.json
//! <profile>/kuru-test-templates/<fingerprint>/store/...
//! ```
//!
//! The fingerprint hashes the supervisor executable that fixture opens use,
//! the schema versions, the pinned engine version, the fixture project scope,
//! the capture format and the sources that define the stored schema, so a
//! changed engine, supervisor or migration set never reuses an old template.
//!
//! Correctness does not rely on process-local state. Creation, validation and
//! rebuilding hold the fingerprint's exclusive file lock; every copy holds its
//! shared lock. A template is staged in a private sibling, verified, and only
//! then moved to its final name, so a partial template is never observed. A
//! template that does not match its manifest is removed and rebuilt under the
//! exclusive lock; it is never accepted.
//!
//! Capture begins only after the cold store closed and the lifecycle lease was
//! acquired, which proves that its supervisor reaped Dolt. Lifecycle, startup
//! and engine lock files, the server configuration and log, and staging
//! directories are excluded; an endpoint, migration record, Dolt server info,
//! PID or socket file means the store was not cleanly closed and refuses the
//! capture. Every object is copied through checked private-directory handles:
//! links are refused, and each directory and file is created fresh as
//! owner-only (a protected DACL on Windows) instead of inheriting source
//! access. Each per-test copy is verified against the manifest's paths, sizes
//! and SHA-256 digests while it is written.
//!
//! [`MemoryStore::temporary`]: crate::MemoryStore::temporary
use super::{digest, profile};
use crate::{
    files,
    server::Server,
    store::{Creation, MemoryStore},
};
use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::time::Instant;

/// Bump when the capture rules or the on-disk layout change.
const FORMAT: u32 = 1;
const DIRECTORY: &str = "kuru-test-templates";
const STORE: &str = "store";
const MANIFEST: &str = "manifest.json";
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: usize = 16;
const FILE_LIMIT: u64 = 256 * 1024 * 1024;
/// A waiter covers one peer's complete cold open and capture.
const LOCK_WAIT: Duration = Duration::from_secs(300);
const QUIESCENCE_WAIT: Duration = Duration::from_secs(60);
/// Sources that define the stored schema, its bootstrap and this capture.
const SOURCES: &[&[u8]] = &[
    include_bytes!("../store.rs"),
    include_bytes!("../store/migrations.rs"),
    include_bytes!("../store/usage_ledger.rs"),
    include_bytes!("../server.rs"),
    include_bytes!("template.rs"),
];

/// How a template-backed open obtained its template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Reused,
    Created,
    Rebuilt,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
enum Entry {
    Directory {
        path: Vec<String>,
    },
    File {
        path: Vec<String>,
        bytes: u64,
        sha256: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    fingerprint: String,
    entries: Vec<Entry>,
}

struct Inputs<'a> {
    supervisor: &'a str,
    schema: (i32, i32),
    engine: &'a str,
    scope: &'a str,
    sources: &'a str,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn compose(inputs: &Inputs<'_>) -> String {
    let mut hash = Sha256::new();
    for field in [
        b"kuru-memory-test-template".as_slice(),
        &FORMAT.to_be_bytes(),
        inputs.supervisor.as_bytes(),
        &inputs.schema.0.to_be_bytes(),
        &inputs.schema.1.to_be_bytes(),
        inputs.engine.as_bytes(),
        inputs.scope.as_bytes(),
        inputs.sources.as_bytes(),
        std::env::consts::OS.as_bytes(),
        std::env::consts::ARCH.as_bytes(),
    ] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    hex(&hash.finalize())
}

fn sources() -> &'static str {
    static SOURCES_DIGEST: OnceLock<String> = OnceLock::new();
    SOURCES_DIGEST.get_or_init(|| {
        let mut hash = Sha256::new();
        for source in SOURCES {
            hash.update((source.len() as u64).to_be_bytes());
            hash.update(source);
        }
        hex(&hash.finalize())
    })
}

/// SHA-256 of the supervisor executable fixture opens actually launch: the
/// prepared snapshot, or the instrumented helper under coverage.
fn supervisor() -> Result<String> {
    static SUPERVISOR: OnceLock<Result<String, String>> = OnceLock::new();
    SUPERVISOR
        .get_or_init(|| {
            (|| {
                let path = crate::store::test_supervisor()?;
                let mut file = File::open(&path)
                    .with_context(|| format!("open test supervisor {}", path.display()))?;
                ensure!(
                    file.metadata()?.is_file(),
                    "test supervisor is not a regular file"
                );
                Ok(digest(&mut file)?.1)
            })()
            .map_err(|error: anyhow::Error| format!("fingerprint the test supervisor: {error:#}"))
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

fn fingerprint(scope: &str) -> Result<String> {
    Ok(compose(&Inputs {
        supervisor: &supervisor()?,
        schema: crate::store::template_schema(),
        engine: crate::provision::DOLT_VERSION,
        scope,
        sources: sources(),
    }))
}

fn default_root() -> Result<PathBuf> {
    Ok(profile(&std::env::current_exe()?)?.join(DIRECTORY))
}

/// Create `data`'s project store for `scope` as a private copy of the current
/// template, first creating or rebuilding that template when necessary.
pub(crate) async fn instantiate(data: &Path, scope: &str) -> Result<Outcome> {
    instantiate_in(&default_root()?, data, scope).await
}

/// Releases an advisory lock that this handle holds.
struct Held<'a>(&'a File);

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

async fn acquire<'a>(lock: &'a File, shared: bool, path: &Path) -> Result<Held<'a>> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let attempt = if shared {
            lock.try_lock_shared()
        } else {
            lock.try_lock()
        };
        match attempt {
            Ok(()) => return Ok(Held(lock)),
            Err(TryLockError::WouldBlock) => {
                ensure!(
                    Instant::now() < deadline,
                    "memory test template lock {} stayed held for {LOCK_WAIT:?}; another test process may still be creating the template",
                    path.display()
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("lock memory test template {}", path.display()));
            }
        }
    }
}

pub(crate) async fn instantiate_in(root: &Path, data: &Path, scope: &str) -> Result<Outcome> {
    let fingerprint = fingerprint(scope)?;
    let root = files::ensure_private_directory(root)?;
    let lock_name = format!("{fingerprint}.lock");
    let lock_path = root.path().join(&lock_name);
    let lock = root.lock_file(OsStr::new(&lock_name))?;
    let template = root.path().join(&fingerprint);
    let destination = crate::store::project_directory(data, scope)?;
    let mut outcome = Outcome::Reused;
    let mut prepared = false;
    loop {
        if prepared || fs::symlink_metadata(template.join(MANIFEST)).is_ok() {
            let copied = {
                let _held = acquire(&lock, true, &lock_path).await?;
                root.verify(OsStr::new(&lock_name), &lock)?;
                copy_template(&template, &fingerprint, &destination)
            };
            match copied {
                Ok(()) => return Ok(outcome),
                Err(error) if prepared => {
                    return Err(error.context(format!(
                        "copy the validated memory test template {}",
                        template.display()
                    )));
                }
                // Validation under the exclusive lock decides whether the
                // template itself is invalid; this copy is simply discarded.
                Err(_) => discard(&destination)?,
            }
        }
        {
            let _held = acquire(&lock, false, &lock_path).await?;
            root.verify(OsStr::new(&lock_name), &lock)?;
            outcome = prepare(&root, &fingerprint, scope).await?;
        }
        prepared = true;
    }
}

fn discard(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Ok(files::directory(destination)?.remove_tree()?),
    }
}

/// Under the exclusive lock: keep a valid template, or build a replacement.
async fn prepare(root: &Directory, fingerprint: &str, scope: &str) -> Result<Outcome> {
    let template = root.path().join(fingerprint);
    let outcome = match fs::symlink_metadata(&template) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Outcome::Created,
        Err(error) => return Err(error.into()),
        Ok(_) => {
            if validate(&template, fingerprint).is_ok() {
                return Ok(Outcome::Reused);
            }
            files::directory(&template)
                .and_then(|directory| Ok(directory.remove_tree()?))
                .with_context(|| {
                    format!("remove invalid memory test template {}", template.display())
                })?;
            Outcome::Rebuilt
        }
    };
    // Only an exclusive holder builds, so earlier stages were abandoned.
    let stage_prefix = format!("{fingerprint}.stage-");
    for entry in fs::read_dir(root.path())? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&stage_prefix))
        {
            files::directory(&entry.path())?.remove_tree()?;
        }
    }
    build(root, fingerprint, scope).await?;
    Ok(outcome)
}

async fn build(root: &Directory, fingerprint: &str, scope: &str) -> Result<()> {
    let cold = Arc::new(
        tempfile::Builder::new()
            .prefix("kuru-memory-template-")
            .tempdir()?,
    );
    let data = cold.path().join("private");
    // No fixture permit: template-backed callers acquire theirs after copying.
    // Boxed: the complete cold-open future is far larger than a copy.
    let store = Box::pin(MemoryStore::open_temporary(
        cold.clone(),
        None,
        Creation::Default,
    ))
    .await
    .context("cold-open the memory test template source")?;
    store
        .close()
        .await
        .context("close the memory test template source")?;
    let source = crate::store::project_directory(&data, scope)?;
    #[cfg(unix)]
    let lifecycle_root: Option<PathBuf> = None;
    #[cfg(windows)]
    let lifecycle_root = Some(data.join("memory/lifecycles"));
    // Holding the lifecycle lease proves that the supervisor reaped Dolt.
    let lease = Server::quiescence_at(&source, lifecycle_root.as_deref(), QUIESCENCE_WAIT)
        .await
        .context("memory test template source did not quiesce")?;
    let stage_name = format!("{fingerprint}.stage-{}", uuid::Uuid::new_v4());
    let stage = root.create_private_directory(OsStr::new(&stage_name))?;
    let staged = (|| -> Result<()> {
        let store = stage.create_private_directory(OsStr::new(STORE))?;
        let entries = walk(&source, Walk::Capture, Some(&store))?;
        let manifest = Manifest {
            format: FORMAT,
            fingerprint: fingerprint.to_owned(),
            entries,
        };
        let mut file = stage.create_new(OsStr::new(MANIFEST))?;
        file.write_all(&serde_json::to_vec(&manifest)?)?;
        file.sync_all()?;
        drop(file);
        validate(stage.path(), fingerprint)
    })();
    drop(lease);
    match staged {
        Ok(()) => {
            files::move_directory(&stage, &root.path().join(fingerprint))
                .context("publish the memory test template")?;
        }
        Err(error) => {
            if let Err(cleanup) = stage.remove_tree() {
                return Err(error.context(format!(
                    "memory test template stage cleanup also failed: {cleanup}"
                )));
            }
            return Err(error.context("capture the memory test template"));
        }
    }
    drop(cold);
    Ok(())
}

fn read_manifest(template: &Path, fingerprint: &str) -> Result<Manifest> {
    let bytes = files::read_bytes(&template.join(MANIFEST), MANIFEST_LIMIT)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("invalid memory test template manifest")?;
    ensure!(
        manifest.format == FORMAT
            && manifest.fingerprint == fingerprint
            && !manifest.entries.is_empty(),
        "memory test template manifest does not match this fixture"
    );
    Ok(manifest)
}

/// The template directory holds exactly its manifest and a store tree that
/// matches it, all owner-private.
fn validate(template: &Path, fingerprint: &str) -> Result<()> {
    let manifest = read_manifest(template, fingerprint)?;
    let mut names = fs::read_dir(template)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    ensure!(
        names == [OsStr::new(MANIFEST), OsStr::new(STORE)],
        "memory test template has unexpected entries"
    );
    let entries = walk(&template.join(STORE), Walk::Template, None)?;
    ensure!(
        entries == manifest.entries,
        "memory test template does not match its manifest"
    );
    Ok(())
}

fn copy_template(template: &Path, fingerprint: &str, destination: &Path) -> Result<()> {
    let manifest = read_manifest(template, fingerprint)?;
    let memory = destination
        .parent()
        .context("memory project store has no parent")?;
    let data = memory.parent().context("memory directory has no parent")?;
    files::private_dir(data)?;
    let memory = files::ensure_private_directory(memory)?;
    let store = memory.create_private_directory(files::name(destination)?)?;
    let entries = walk(&template.join(STORE), Walk::Template, Some(&store))?;
    ensure!(
        entries == manifest.entries,
        "memory test template does not match its manifest"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Walk {
    /// Read a cleanly closed store, excluding runtime state.
    Capture,
    /// Read an owner-private template that contains no runtime state.
    Template,
}

#[derive(Debug, Eq, PartialEq)]
enum Class {
    Copy,
    Skip,
}

fn runtime_lock(name: &str) -> bool {
    name == "LOCK" || name.ends_with(".lock")
}

fn unclean(name: &str) -> bool {
    matches!(name, "endpoint.json" | "migration.json" | "sql-server.info")
        || name.ends_with(".pid")
        || name.ends_with(".sock")
}

fn classify(mode: Walk, path: &[String], directory: bool, object: &Path) -> Result<Class> {
    let name = path.last().context("empty template path")?.as_str();
    let display = path.join("/");
    ensure!(
        !unclean(name),
        "memory test template source was not cleanly closed: {display}"
    );
    if path.len() == 1 {
        return match (name, directory, mode) {
            ("identity.json" | "ready.json", false, _) | ("data" | "config" | "home", true, _) => {
                Ok(Class::Copy)
            }
            ("lifecycle.lock" | "server.log" | "server.yaml", false, Walk::Capture) => {
                Ok(Class::Skip)
            }
            ("staging", true, Walk::Capture) => {
                ensure!(
                    fs::read_dir(object)?.next().is_none(),
                    "memory test template source has pending staged records"
                );
                Ok(Class::Skip)
            }
            _ => bail!("memory test template has an unrecognized store entry: {display}"),
        };
    }
    if runtime_lock(name) {
        ensure!(
            mode == Walk::Capture && !directory,
            "memory test template contains runtime lock state: {display}"
        );
        return Ok(Class::Skip);
    }
    Ok(Class::Copy)
}

fn walk(source: &Path, mode: Walk, destination: Option<&Directory>) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    walk_into(
        source,
        Privacy::OwnerOnly,
        mode,
        destination,
        &mut Vec::new(),
        &mut entries,
    )
    .with_context(|| format!("read memory test template tree {}", source.display()))?;
    Ok(entries)
}

fn walk_into(
    source: &Path,
    privacy: Privacy,
    mode: Walk,
    destination: Option<&Directory>,
    path: &mut Vec<String>,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    ensure!(
        path.len() < MAX_DEPTH,
        "memory test template is nested too deeply"
    );
    let directory = files::open_directory(source, privacy, NameRetention::Movable)?;
    let mut names = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    // Dolt creates its own objects with ordinary modes below the private
    // store root; capture reads them through checked inherited-access handles.
    // Templates and their copies are created, and read back, owner-only.
    let child_privacy = match mode {
        Walk::Capture => Privacy::Inherited,
        Walk::Template => Privacy::OwnerOnly,
    };
    for name in names {
        let text = name
            .to_str()
            .with_context(|| format!("non-UTF-8 memory test template entry {name:?}"))?
            .to_owned();
        let object = source.join(&name);
        let kind = fs::symlink_metadata(&object)?.file_type();
        ensure!(
            !kind.is_symlink(),
            "memory test template refuses link {}",
            object.display()
        );
        path.push(text);
        if classify(mode, path, kind.is_dir(), &object)? == Class::Copy {
            ensure!(
                entries.len() < MAX_ENTRIES,
                "memory test template has too many entries"
            );
            if kind.is_dir() {
                let child = destination
                    .map(|parent| parent.create_private_directory(&name))
                    .transpose()?;
                entries.push(Entry::Directory { path: path.clone() });
                walk_into(&object, child_privacy, mode, child.as_ref(), path, entries)?;
            } else if kind.is_file() {
                // Checked opens refuse links, including extra hard links.
                let mut input = directory.read(&name)?;
                let mut output = destination
                    .map(|parent| parent.create_new(&name))
                    .transpose()?;
                let (bytes, sha256) = copy_hashing(&mut input, output.as_mut())?;
                directory.verify(&name, &input)?;
                if let Some(output) = output
                    && mode == Walk::Capture
                {
                    output.sync_all()?;
                }
                entries.push(Entry::File {
                    path: path.clone(),
                    bytes,
                    sha256,
                });
            } else {
                bail!(
                    "memory test template refuses non-regular object {}",
                    object.display()
                );
            }
        }
        path.pop();
    }
    directory.revalidate()?;
    Ok(())
}

fn copy_hashing(input: &mut File, mut output: Option<&mut File>) -> Result<(u64, String)> {
    let mut limited = input.take(FILE_LIMIT + 1);
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let read = limited.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        ensure!(
            count <= FILE_LIMIT,
            "memory test template file exceeds size limit"
        );
        hash.update(&buffer[..read]);
        if let Some(output) = output.as_deref_mut() {
            output.write_all(&buffer[..read])?;
        }
    }
    Ok((count, hex(&hash.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryStore as Store, OpenOptions};

    const CHILD_ROOT: &str = "KURU_TEST_TEMPLATE_CHILD_ROOT";
    const CHILD_DATA: &str = "KURU_TEST_TEMPLATE_CHILD_DATA";
    const CHILD_OUTCOME: &str = "KURU_TEST_TEMPLATE_CHILD_OUTCOME";
    const CHILD_TEST: &str =
        "test_support::template::tests::child_process_instantiates_the_named_template";
    const CHILD_DEADLINE: Duration = Duration::from_secs(360);
    /// The fixture guard's depth budget for a container that keeps a
    /// template and unopened copies: the deepest is a copy's
    /// `<name>/private/memory/<hash>/data/kuru/.dolt/stats/.dolt/noms/oldgen`,
    /// 11 levels below the container.
    const TEMPLATE_FIXTURE_DEPTH: usize = 11;

    fn scope() -> String {
        crate::store::temporary_scope()
    }

    fn top_level(manifest: &Manifest) -> Vec<(String, bool)> {
        manifest
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Directory { path } if path.len() == 1 => Some((path[0].clone(), true)),
                Entry::File { path, .. } if path.len() == 1 => Some((path[0].clone(), false)),
                _ => None,
            })
            .collect()
    }

    /// Open a copied store through the ordinary existing-store path.
    async fn open_copy(data: &Path) -> Result<()> {
        let options: OpenOptions = crate::test_support::open_options(data.to_owned(), scope())?;
        let store = crate::test_support::open_local_fixture(options).await?;
        store.close().await
    }

    #[test]
    fn fingerprint_changes_with_every_input() {
        let base = Inputs {
            supervisor: "supervisor",
            schema: (7, 4),
            engine: "2.3.3",
            scope: "project/0",
            sources: "sources",
        };
        let variants = [
            Inputs {
                supervisor: "rebuilt supervisor",
                ..base
            },
            Inputs {
                schema: (8, 4),
                ..base
            },
            Inputs {
                schema: (7, 5),
                ..base
            },
            Inputs {
                engine: "2.3.4",
                ..base
            },
            Inputs {
                scope: "project/1",
                ..base
            },
            Inputs {
                sources: "edited migrations",
                ..base
            },
        ];
        let mut keys = vec![compose(&base)];
        keys.extend(variants.iter().map(compose));
        assert_eq!(compose(&base), keys[0], "fingerprint is not deterministic");
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "an input did not change the key");
        assert!(keys.iter().all(|key| key.len() == 64));
    }

    #[tokio::test]
    async fn shared_template_holds_only_clean_private_store_state() -> Result<()> {
        // Ensures the shared template exists, then inspects it read-only.
        let store = Store::temporary().await?;
        store.close().await?;
        let fingerprint = fingerprint(&scope())?;
        // Read under the shared lock, as every copy does.
        let root = files::ensure_private_directory(&default_root()?)?;
        let lock_name = format!("{fingerprint}.lock");
        let lock = root.lock_file(OsStr::new(&lock_name))?;
        let _held = acquire(&lock, true, &root.path().join(&lock_name)).await?;
        root.verify(OsStr::new(&lock_name), &lock)?;
        let template = root.path().join(&fingerprint);
        let manifest = read_manifest(&template, &fingerprint)?;
        assert_eq!(
            top_level(&manifest),
            [
                ("config".into(), true),
                ("data".into(), true),
                ("home".into(), true),
                ("identity.json".into(), false),
                ("ready.json".into(), false),
            ]
        );
        for entry in &manifest.entries {
            let (Entry::Directory { path } | Entry::File { path, .. }) = entry;
            let name = path.last().map(String::as_str).unwrap_or_default();
            ensure!(
                !runtime_lock(name)
                    && !unclean(name)
                    && !matches!(name, "server.log" | "server.yaml"),
                "template retained runtime state {}",
                path.join("/")
            );
        }
        ensure!(
            manifest.entries.iter().any(|entry| matches!(
                entry,
                Entry::File { path, bytes, .. } if path.first().is_some_and(|name| name == "data") && *bytes > 0
            )),
            "template retained no Dolt data"
        );
        // Every object is owner-private and matches the manifest.
        validate(&template, &fingerprint)
    }

    #[tokio::test]
    async fn invalid_templates_are_rejected_and_rebuilt() -> Result<()> {
        // Keeps a template and unopened copies, which no lease recognises,
        // so their Dolt repositories are scanned in full (11 levels deep).
        let container = crate::test_support::tempdir()?.with_depth_budget(TEMPLATE_FIXTURE_DEPTH);
        let root = container.path().join("templates");
        let data = |name: &str| container.path().join(name).join("private");
        assert_eq!(
            instantiate_in(&root, &data("first"), &scope()).await?,
            Outcome::Created
        );
        assert_eq!(
            instantiate_in(&root, &data("second"), &scope()).await?,
            Outcome::Reused
        );
        let fingerprint = fingerprint(&scope())?;
        let template = root.join(&fingerprint);
        let original = read_manifest(&template, &fingerprint)?;

        // Changed bytes of the same length.
        let identity = template.join(STORE).join("identity.json");
        let mut bytes = fs::read(&identity)?;
        let last = bytes.last_mut().context("empty template identity")?;
        *last ^= 1;
        fs::write(&identity, &bytes)?;
        let error = validate(&template, &fingerprint).expect_err("corrupt template accepted");
        assert!(format!("{error:#}").contains("does not match its manifest"));
        assert_eq!(
            instantiate_in(&root, &data("corrupt"), &scope()).await?,
            Outcome::Rebuilt
        );
        let rebuilt = read_manifest(&template, &fingerprint)?;
        assert_ne!(
            rebuilt.entries, original.entries,
            "rebuild reused old state"
        );
        open_copy(&data("corrupt")).await?;

        // An unpublished manifest and an abandoned stage.
        fs::remove_file(template.join(MANIFEST))?;
        let stage = root.join(format!("{fingerprint}.stage-{}", uuid::Uuid::new_v4()));
        files::private_dir(&stage.join(STORE))?;
        assert_eq!(
            instantiate_in(&root, &data("partial"), &scope()).await?,
            Outcome::Rebuilt
        );
        ensure!(!stage.exists(), "abandoned template stage was retained");
        let mut names = fs::read_dir(&root)?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        assert_eq!(names, [fingerprint.clone(), format!("{fingerprint}.lock")]);
        validate(&template, &fingerprint)?;

        // A second hard link to a template file, outside the template.
        let alias = container.path().join("alias");
        fs::hard_link(template.join(STORE).join("identity.json"), &alias)?;
        let error = validate(&template, &fingerprint).expect_err("hard link accepted");
        assert!(
            format!("{error:#}").contains("exactly one hardlink"),
            "unexpected validation error: {error:#}"
        );
        let copy = container.path().join("hard-link-copy").join("memory");
        files::private_dir(&copy)?;
        let error = copy_template(&template, &fingerprint, &copy.join("store"))
            .expect_err("hard link copied");
        assert!(
            format!("{error:#}").contains("exactly one hardlink"),
            "unexpected copy error: {error:#}"
        );
        assert_eq!(
            instantiate_in(&root, &data("hard-linked"), &scope()).await?,
            Outcome::Rebuilt
        );
        validate(&template, &fingerprint)?;
        open_copy(&data("hard-linked")).await?;

        // A copy never follows or adopts a symbolic link.
        #[cfg(unix)]
        {
            let outside = container.path().join("outside");
            files::write(&outside, b"outside the template")?;
            std::os::unix::fs::symlink(&outside, template.join(STORE).join("data").join("link"))?;
            let error = validate(&template, &fingerprint).expect_err("link accepted");
            assert!(format!("{error:#}").contains("refuses link"));
            let copy = container.path().join("linked").join("memory");
            files::private_dir(&copy)?;
            let error = copy_template(&template, &fingerprint, &copy.join("store"))
                .expect_err("link copied");
            assert!(format!("{error:#}").contains("refuses link"));
        }
        Ok(())
    }

    /// Runs only as the child of `concurrent_processes_create_one_template`.
    #[tokio::test]
    async fn child_process_instantiates_the_named_template() -> Result<()> {
        let (Some(root), Some(data), Some(outcome)) = (
            std::env::var_os(CHILD_ROOT),
            std::env::var_os(CHILD_DATA),
            std::env::var_os(CHILD_OUTCOME),
        ) else {
            return Ok(());
        };
        let result = instantiate_in(Path::new(&root), Path::new(&data), &scope()).await?;
        files::write(Path::new(&outcome), format!("{result:?}").as_bytes())
    }

    #[cfg(unix)]
    async fn spawn_child(
        root: &Path,
        data: &Path,
        outcome: &Path,
        log: &Path,
    ) -> Result<tokio::process::Child> {
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env(CHILD_ROOT, root)
            .env(CHILD_DATA, data)
            .env(CHILD_OUTCOME, outcome)
            .env("KURU_DOLT_CACHE", crate::store::test_cache())
            .env("TMPDIR", std::env::temp_dir())
            .stdin(std::process::Stdio::null())
            .stdout(File::create(log)?)
            .stderr(File::create(log.with_extension("stderr"))?)
            .kill_on_drop(true);
        for name in ["KURU_TEST_SUPERVISOR_PREPARED", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in crate::test_support::lifecycle_trace::forwarded() {
            command.env(name, value);
        }
        // Held across the spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        Ok(command.spawn()?)
    }

    #[cfg(unix)]
    async fn wait_child(mut child: tokio::process::Child) -> Result<std::process::ExitStatus> {
        tokio::time::timeout(CHILD_DEADLINE, child.wait())
            .await
            .context("template child process exceeded its deadline")?
            .map_err(Into::into)
    }

    #[cfg(windows)]
    async fn spawn_child(
        root: &Path,
        data: &Path,
        outcome: &Path,
        log: &Path,
    ) -> Result<kuru_platform::windows::process::NativeChild> {
        use kuru_platform::windows::process::{Console, NativeSpawnSpec, Stdio as NativeStdio};
        let system = kuru_platform::windows::process::system_directory()?;
        let windows = system
            .parent()
            .context("Windows system directory has no parent")?;
        let mut command = NativeSpawnSpec::new(
            std::env::current_exe()?,
            root.parent()
                .context("template root has no parent")?
                .to_owned(),
        );
        command.args = ["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"]
            .into_iter()
            .map(Into::into)
            .collect();
        command.environment = vec![
            ("SystemRoot".into(), windows.as_os_str().into()),
            ("PATH".into(), system.into_os_string()),
            (CHILD_ROOT.into(), root.as_os_str().into()),
            (CHILD_DATA.into(), data.as_os_str().into()),
            (CHILD_OUTCOME.into(), outcome.as_os_str().into()),
            (
                "KURU_DOLT_CACHE".into(),
                crate::store::test_cache().into_os_string(),
            ),
        ];
        for name in [
            "KURU_TEST_SUPERVISOR_PREPARED",
            "LLVM_PROFILE_FILE",
            "TMP",
            "TEMP",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.environment.push((name.into(), value));
            }
        }
        command
            .environment
            .extend(crate::test_support::lifecycle_trace::forwarded());
        let stdout: std::os::windows::io::OwnedHandle = File::create(log)?.into();
        let stderr: std::os::windows::io::OwnedHandle =
            File::create(log.with_extension("stderr"))?.into();
        command.stdin = NativeStdio::Null;
        command.stdout = NativeStdio::Handle(stdout);
        command.stderr = NativeStdio::Handle(stderr);
        command.console = Console::PrivateHidden;
        let _gate = crate::spawn_gate::spawning().await;
        Ok(command.spawn().await?)
    }

    #[cfg(windows)]
    async fn wait_child(
        mut child: kuru_platform::windows::process::NativeChild,
    ) -> Result<std::process::ExitStatus> {
        Ok(child.wait(CHILD_DEADLINE).await?)
    }

    #[tokio::test]
    async fn concurrent_processes_create_one_template() -> Result<()> {
        // Keeps a template and unopened copies, which no lease recognises,
        // so their Dolt repositories are scanned in full (11 levels deep).
        let container = crate::test_support::tempdir()?.with_depth_budget(TEMPLATE_FIXTURE_DEPTH);
        let root = container.path().join("templates");
        let child_data = container.path().join("child").join("private");
        let outcome = container.path().join("child-outcome");
        let log = container.path().join("child.log");
        let child = spawn_child(&root, &child_data, &outcome, &log).await?;
        let parent = instantiate_in(
            &root,
            &container.path().join("parent").join("private"),
            &scope(),
        )
        .await;
        let status = wait_child(child).await?;
        let diagnostics = || {
            format!(
                "child stdout: {}\nchild stderr: {}",
                fs::read_to_string(&log).unwrap_or_default(),
                fs::read_to_string(log.with_extension("stderr")).unwrap_or_default()
            )
        };
        ensure!(
            status.success(),
            "template child failed ({status}): {}",
            diagnostics()
        );
        let parent = parent?;
        let child = String::from_utf8(files::read_bytes(&outcome, 64)?)?;
        let mut outcomes = [format!("{parent:?}"), child];
        outcomes.sort();
        assert_eq!(
            outcomes,
            ["Created", "Reused"],
            "one process creates and the other reuses: {}",
            diagnostics()
        );
        let fingerprint = fingerprint(&scope())?;
        let mut names = fs::read_dir(&root)?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        assert_eq!(names, [fingerprint.clone(), format!("{fingerprint}.lock")]);
        validate(&root.join(&fingerprint), &fingerprint)?;
        open_copy(&child_data).await?;
        open_copy(&container.path().join("parent").join("private")).await
    }
}
