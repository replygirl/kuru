//! The per-machine store template cache.
//!
//! A store template is the `data/` of a store built once per machine and key
//! by the real migration chain, under the compiled placeholder identity. It
//! lives beside the engine it was built with:
//!
//! ```text
//! <cache>/<DOLT_VERSION>/templates/
//!   <key>.lock                      key lock; permanent, never removed
//!   <key>/manifest.json             format, key, entries (path, bytes, SHA-256)
//!   <key>/data/...                  the captured data/ only
//!   .build-<key>-<uuid>/store/...   a build store (transient)
//!   .stage-<key>-<uuid>/            a capture stage (transient)
//!   .rejected-<key>-<uuid>/         a quarantined template, at most one per key
//!   lifecycles/                     Windows only: build-store lifecycle leases
//! ```
//!
//! The root is a parameter of every entry point ([`ensure_in`], [`create_in`]):
//! product code passes [`root_in`] of its engine cache, and tests that need an
//! empty or damaged root pass a private one.
//!
//! **Key.** [`compiled_key`] hashes, with length framing, exactly what
//! determines a fresh store's bytes: the template format, the pinned engine
//! version, this target's pinned engine digest and triple, both schema
//! versions, the receipt digest of every registry definition in order, and
//! the creation statements, placeholder literals, commit-message formats and
//! `server.yaml` behavior that shape an empty store. Supervisor bytes and
//! source digests are excluded, so an instrumented and an ordinary supervisor
//! build interchangeable templates.
//!
//! **Build.** Only the holder of the key's exclusive lock builds. It first
//! sweeps this key's abandoned build and capture stages without waiting: one
//! quiescence attempt at the smallest valid wait, and an entry whose lease is
//! still held, or whose removal fails, is left for a later holder. The build
//! itself is one stage-worker job on one engine (bootstrap under the
//! placeholder identity, initialization, the chain, the usage branch's chain
//! and validation, validation of `main`, and the shared template shape) with
//! the key lock as the engine's reap guard. After the engine is reaped,
//! `data/` alone is captured through checked private handles (runtime locks
//! skipped; endpoint, migration, server-info, PID and socket files refuse the
//! capture; every file and directory synced), byte-scanned for the build
//! store's path, both build secrets and the host name, described by a synced
//! manifest, verified, and published by a no-replace rename with a bounded
//! retry. A capture stage that fails its scan or its verification is removed
//! at once (best-effort; a failed removal is left for the next exclusive
//! holder's sweep). A capture stage that cannot be published stays verified
//! for its builder's own use and is swept by the next exclusive holder.
//!
//! The byte scan finds literal occurrences only. Dolt stores chunks
//! compressed, and a value repeated within a chunk can become a
//! back-reference (engine contract finding S7), so the scan is defence in
//! depth: the guarantee rests on the shape assertion, which checks at the SQL
//! level that the template holds schema, receipts and the placeholder row,
//! and nothing else, and that every commit on both refs carries the engine's
//! fixed system account or Kuru's compiled author and the compiled messages.
//! Host names shorter than [`HOSTNAME_NEEDLE_MIN`] are not scanned for: a
//! two- or three-byte sequence occurs by chance in compressed chunks and
//! would refuse sound builds. Nor is the operating-system user name scanned,
//! for the same reason (short and common names such as `rg` or `data`): the
//! engine runs with a cleared environment and a private home, and the commit
//! assertions are what rule out a recorded identity.
//!
//! **Use.** A copier takes the shared key lock and never waits: the
//! structural check ([`Judged`]) reads the manifest and the top-level names,
//! and [`copy_into`] verifies every file's size and digest while it writes,
//! refuses links, extra hard links and other object types, syncs every file
//! and directory and compares the entry set.
//!
//! **Quarantine** follows only a verdict against the bytes: a completed
//! comparison that returned another value ([`CreationFailure::TemplateVerdict`]).
//! Engine, SQL, I/O, lock and deadline failures never touch a template. A
//! quarantine is bound to the directory that was judged: under the exclusive
//! lock the published name must still hold the judged identity, or nothing is
//! moved. It is best-effort: a busy lock, another identity or a failed rename
//! leaves the template where it is, with a warning.
//!
//! **No garbage collection.** Nothing here removes another key's template,
//! another key's quarantined directory or any lock file. Lock files are
//! permanent, as old engine versions are.
use super::*;
use crate::server::{TEMPLATE_SCOPE, TemplateVerdict};
use anyhow::anyhow;
use kuru_platform::fs::FileIdentity;
use std::{
    ffi::OsStr,
    fmt,
    fs::TryLockError,
    io::{Read, Write},
    sync::OnceLock,
};

/// Bump when the capture rules, the placeholder literals, the adoption
/// protocol, the shape assertions or the on-disk layout change.
pub(crate) const TEMPLATE_FORMAT: u32 = 1;
const KEY_DOMAIN: &str = "kuru-memory-store-template";
/// The templates directory beside an engine version's published engine.
pub(crate) const TEMPLATES: &str = "templates";
const MANIFEST: &str = "manifest.json";
const DATA: &str = "data";
const BUILD_STORE: &str = "store";
#[cfg(windows)]
const LIFECYCLES: &str = "lifecycles";
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: usize = 16;
const FILE_LIMIT: u64 = 256 * 1024 * 1024;
/// The most `templates/` entries a sweep reads.
const ROOT_ENTRY_LIMIT: usize = 4096;
/// The smallest valid quiescence wait (zero is refused): a sweep tries once.
const SWEEP_QUIESCENCE: Duration = Duration::from_millis(10);
/// The bounded publication retry, as the engine activation retries.
const PUBLISH_RETRY_LIMIT: Duration = Duration::from_secs(2);
const PUBLISH_RETRY_SPACING: Duration = Duration::from_millis(20);
/// How often a waiting warm-up retries a busy key lock.
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "only test-fixture warm-up and prefetch wait for or ensure a template; an open uses create_in"
    )
)]
const LOCK_POLL: Duration = Duration::from_millis(25);
/// The shortest host name the byte scan looks for: shorter names occur by
/// chance in compressed chunks and would refuse sound builds.
const HOSTNAME_NEEDLE_MIN: usize = 4;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// --- Key ---------------------------------------------------------------------

/// Everything a template's bytes depend on.
#[derive(Clone, Copy)]
struct KeyInputs<'a> {
    format: u32,
    engine: &'a str,
    executable: &'a str,
    target: &'a str,
    schema: i32,
    usage_schema: i32,
    /// The format of a migration publication record, which a template's
    /// `main` carries one of per retained attempt.
    record_format: i8,
    /// Receipt digests of the main and the usage registries, in order.
    definitions: &'a [Vec<String>; 2],
    statements: &'a [&'a str],
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn compose(inputs: &KeyInputs<'_>) -> String {
    let mut hash = Sha256::new();
    frame(&mut hash, KEY_DOMAIN.as_bytes());
    frame(&mut hash, &inputs.format.to_be_bytes());
    frame(&mut hash, inputs.engine.as_bytes());
    frame(&mut hash, inputs.executable.as_bytes());
    frame(&mut hash, inputs.target.as_bytes());
    frame(&mut hash, &inputs.schema.to_be_bytes());
    frame(&mut hash, &inputs.usage_schema.to_be_bytes());
    frame(&mut hash, &inputs.record_format.to_be_bytes());
    for registry in inputs.definitions {
        frame(&mut hash, &(registry.len() as u64).to_be_bytes());
        for definition in registry {
            frame(&mut hash, definition.as_bytes());
        }
    }
    frame(&mut hash, &(inputs.statements.len() as u64).to_be_bytes());
    for statement in inputs.statements {
        frame(&mut hash, statement.as_bytes());
    }
    hex(&hash.finalize())
}

/// The creation statements, literals and settings that shape an empty
/// store's bytes, in a fixed order: the bootstrap statements and placeholder
/// literals (the reader secret excluded), the `server.yaml` behavior block,
/// initialization and its commit, the commit author, and the migration
/// path's statements, message format and branch names.
fn creation_statements() -> Vec<&'static str> {
    let [reader_before, reader_after] = crate::server::BOOTSTRAP_READER_ACCOUNT;
    let mut statements = vec![
        crate::server::BOOTSTRAP_CREATE_DATABASE,
        crate::server::BOOTSTRAP_CREATE_IDENTITY,
        crate::server::BOOTSTRAP_COUNT_IDENTITY,
        crate::server::BOOTSTRAP_INSERT_IDENTITY,
        reader_before,
        reader_after,
        crate::server::BOOTSTRAP_READER_GRANT,
        crate::server::TEMPLATE_INSTANCE,
        crate::server::TEMPLATE_SCOPE,
        crate::server::SERVER_BEHAVIOR,
    ];
    statements.extend(INITIALIZE_STATEMENTS);
    statements.push(INITIALIZE_COMMIT);
    statements.push(AUTHOR);
    statements.extend(migrations::TEMPLATE_KEY_STATEMENTS);
    statements
}

/// The store template key of this build: a lowercase SHA-256, computed once
/// per process from compiled constants.
pub(crate) fn compiled_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| {
        compose(&KeyInputs {
            format: TEMPLATE_FORMAT,
            engine: provision::DOLT_VERSION,
            executable: crate::catalog::BUNDLED_ASSET.executable_sha256,
            target: crate::catalog::BUNDLED_ASSET.target,
            schema: migrations::CURRENT_VERSION,
            usage_schema: migrations::USAGE_CURRENT_VERSION,
            record_format: migrations::PUBLICATION_RECORD_FORMAT,
            definitions: &migrations::template_key_definitions(),
            statements: &creation_statements(),
        })
    })
}

// --- Layout -------------------------------------------------------------------

/// The templates root of an engine cache: `<cache>/<DOLT_VERSION>/templates`.
pub(crate) fn root_in(cache: &Path) -> PathBuf {
    cache.join(provision::DOLT_VERSION).join(TEMPLATES)
}

fn lock_name(key: &str) -> String {
    format!("{key}.lock")
}

fn transient_name(kind: &str, key: &str) -> String {
    format!(".{kind}-{key}-{}", Uuid::new_v4())
}

/// Open (creating privately when absent) a templates root.
fn open_root(root: &Path) -> Result<Directory> {
    files::ensure_private_directory(root)
        .with_context(|| format!("open the store template root {}", root.display()))
}

/// The external lifecycle root of build stores (Windows), outside every
/// moved tree; Unix keeps its lease in the store.
///
/// Each build store leaves one small `<identity>.lock` lease file there.
/// They are lock files, and lock files are permanent here (see "No garbage
/// collection" above): removing a lock file another process may hold open
/// leaves its name delete-pending on Windows. Product project leases under
/// `memory/lifecycles` follow the same rule. A build happens once per key
/// and machine, so the directory grows by one file per build; deleting the
/// whole cache while nothing runs reclaims it.
fn build_lifecycle_root(root: &Directory) -> Result<Option<PathBuf>> {
    #[cfg(unix)]
    {
        let _ = root;
        Ok(None)
    }
    #[cfg(windows)]
    {
        let lifecycles = root.path().join(LIFECYCLES);
        files::ensure_private_directory(&lifecycles)?;
        Ok(Some(lifecycles))
    }
}

// --- Failures -------------------------------------------------------------------

/// Why creating from, building or verifying a template failed.
///
/// Only [`Self::TemplateVerdict`] is evidence against a template's bytes; it
/// alone may quarantine one, and only the template that was judged. Every
/// other failure leaves every template untouched.
#[derive(Debug)]
pub(crate) enum CreationFailure {
    /// A completed comparison against the template's bytes returned another
    /// value: manifest, names, sizes, digests, entry set, object kinds, the
    /// placeholder row or the template shape.
    TemplateVerdict(anyhow::Error),
    /// The engine or SQL failed, or a deadline elapsed: nothing about the
    /// bytes.
    Engine(anyhow::Error),
    /// A filesystem or lock operation failed: nothing about the bytes.
    Io(anyhow::Error),
}

impl CreationFailure {
    fn verdict(reason: impl Into<String>) -> Self {
        Self::TemplateVerdict(
            TemplateVerdict::new(format!("memory store template: {}", reason.into())).into(),
        )
    }

    fn io(error: impl Into<anyhow::Error>) -> Self {
        Self::Io(error.into())
    }

    /// An engine-side error, unless a [`TemplateVerdict`] is in its chain.
    fn classify(error: anyhow::Error) -> Self {
        if TemplateVerdict::find(&error).is_some() {
            Self::TemplateVerdict(error)
        } else {
            Self::Engine(error)
        }
    }

    pub(crate) fn is_verdict(&self) -> bool {
        matches!(self, Self::TemplateVerdict(_))
    }

    fn error(&self) -> &anyhow::Error {
        match self {
            Self::TemplateVerdict(error) | Self::Engine(error) | Self::Io(error) => error,
        }
    }

    /// The underlying error, with any [`TemplateVerdict`] still in its chain.
    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            Self::TemplateVerdict(error) | Self::Engine(error) | Self::Io(error) => error,
        }
    }

    fn context(self, context: String) -> Self {
        match self {
            Self::TemplateVerdict(error) => Self::TemplateVerdict(error.context(context)),
            Self::Engine(error) => Self::Engine(error.context(context)),
            Self::Io(error) => Self::Io(error.context(context)),
        }
    }
}

impl fmt::Display for CreationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::TemplateVerdict(_) => "template verdict",
            Self::Engine(_) => "engine failure",
            Self::Io(_) => "I/O failure",
        };
        write!(formatter, "{kind}: {:#}", self.error())
    }
}

impl std::error::Error for CreationFailure {}

/// A refusal of the object a walk reads: a verdict against a template, or a
/// capture refusal of a build store that was not cleanly closed.
fn refusal(mode: Walk, reason: String) -> CreationFailure {
    match mode {
        Walk::Template => CreationFailure::verdict(reason),
        Walk::Capture => {
            CreationFailure::Engine(anyhow!("memory store template capture refused: {reason}"))
        }
    }
}

/// A checked open refuses links and extra hard links with this text.
fn link_refusal(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        && error.to_string().contains("exactly one hardlink")
}

// --- Locks ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Shared,
    Exclusive,
}

/// Open the key's permanent lock file and take it in `mode` without waiting.
/// `None` when another holder excludes it; an open, lock or identity error is
/// an error, which a product opener treats as "no template" and warm-up as
/// fatal.
fn try_key_lock(root: &Directory, key: &str, mode: Mode) -> Result<Option<File>> {
    let name = lock_name(key);
    let name = OsStr::new(&name);
    #[cfg(test)]
    hooks::lock_fault(hooks::LockStep::Open)?;
    let file = root
        .lock_file(name)
        .context("open the store template key lock")?;
    #[cfg(test)]
    let injected = hooks::lock_fault(hooks::LockStep::TryLock).is_err();
    #[cfg(not(test))]
    let injected = false;
    let attempt = if injected {
        Err(TryLockError::Error(std::io::Error::other(
            "injected store template lock failure",
        )))
    } else {
        match mode {
            Mode::Shared => file.try_lock_shared(),
            Mode::Exclusive => file.try_lock(),
        }
    };
    match attempt {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(error)) => {
            return Err(error).context("lock the store template key");
        }
    }
    #[cfg(test)]
    hooks::lock_fault(hooks::LockStep::Verify)?;
    root.verify(name, &file)
        .context("verify the store template key lock")?;
    Ok(Some(file))
}

// --- Manifest and structural check ------------------------------------------------

/// One object below a template's `data/`, by relative path components.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum Entry {
    Directory {
        path: Vec<String>,
    },
    File {
        path: Vec<String>,
        bytes: u64,
        sha256: String,
    },
}

impl Entry {
    fn path(&self) -> String {
        match self {
            Self::Directory { path } | Self::File { path, .. } => path.join("/"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) format: u32,
    pub(crate) key: String,
    pub(crate) entries: Vec<Entry>,
}

/// A template directory that passed the structural check, and the identity
/// of the directory that was judged. A quarantine is bound to that identity.
#[derive(Debug)]
pub(crate) struct Judged {
    pub(crate) path: PathBuf,
    pub(crate) identity: FileIdentity,
    pub(crate) manifest: Manifest,
}

/// The published `<key>/` as one structural check found it.
pub(crate) enum Inspection {
    Absent,
    Valid(Judged),
    /// A verdict against the published bytes, with the identity of the
    /// judged directory when it is a directory at all.
    Condemned {
        identity: Option<FileIdentity>,
        failure: CreationFailure,
    },
}

/// The structural check of `root/<key>`: manifest parse, format and key, and
/// top-level names exactly `{data, manifest.json}`. No hashing. A read error
/// is an error, never a verdict.
fn inspect(root: &Directory, key: &str) -> Result<Inspection, CreationFailure> {
    let path = root.path().join(key);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Inspection::Absent),
        Err(error) => Err(CreationFailure::io(error)),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Ok(Inspection::Condemned {
                identity: None,
                failure: CreationFailure::verdict(format!("{} is not a directory", path.display())),
            })
        }
        Ok(_) => {
            let directory = files::directory(&path).map_err(CreationFailure::Io)?;
            let identity = directory.identity();
            match judge(&directory, key) {
                Ok(manifest) => Ok(Inspection::Valid(Judged {
                    path,
                    identity,
                    manifest,
                })),
                Err(failure) if failure.is_verdict() => Ok(Inspection::Condemned {
                    identity: Some(identity),
                    failure,
                }),
                Err(failure) => Err(failure),
            }
        }
    }
}

fn names(path: &Path) -> Result<Vec<String>, CreationFailure> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(CreationFailure::io)? {
        let entry = entry.map_err(CreationFailure::io)?;
        if names.len() >= MAX_ENTRIES {
            return Err(CreationFailure::verdict(format!(
                "{} holds too many entries",
                path.display()
            )));
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// Read and check the manifest of the template directory `directory`.
fn judge(directory: &Directory, key: &str) -> Result<Manifest, CreationFailure> {
    let found = names(directory.path())?;
    if found != [DATA, MANIFEST] {
        return Err(CreationFailure::verdict(format!(
            "unexpected top-level entries {found:?}"
        )));
    }
    let data = fs::symlink_metadata(directory.path().join(DATA)).map_err(CreationFailure::io)?;
    if data.file_type().is_symlink() || !data.is_dir() {
        return Err(CreationFailure::verdict("data is not a directory"));
    }
    #[cfg(test)]
    hooks::manifest_fault().map_err(CreationFailure::Io)?;
    let mut file =
        directory
            .read(OsStr::new(MANIFEST))
            .map_err(|error| match link_refusal(&error) {
                true => CreationFailure::verdict(format!("the manifest is linked: {error}")),
                false => CreationFailure::io(error),
            })?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MANIFEST_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(CreationFailure::io)?;
    if bytes.len() as u64 > MANIFEST_LIMIT {
        return Err(CreationFailure::verdict(
            "the manifest exceeds its size limit",
        ));
    }
    directory
        .verify(OsStr::new(MANIFEST), &file)
        .map_err(CreationFailure::io)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| CreationFailure::verdict(format!("invalid manifest: {error}")))?;
    if manifest.format != TEMPLATE_FORMAT {
        return Err(CreationFailure::verdict(format!(
            "manifest format {} is not {TEMPLATE_FORMAT}",
            manifest.format
        )));
    }
    if manifest.key != key {
        return Err(CreationFailure::verdict(format!(
            "manifest key {:?} is not {key:?}",
            manifest.key.chars().take(128).collect::<String>()
        )));
    }
    if manifest.entries.is_empty() || manifest.entries.len() > MAX_ENTRIES {
        return Err(CreationFailure::verdict(format!(
            "manifest lists {} entries",
            manifest.entries.len()
        )));
    }
    directory.revalidate().map_err(CreationFailure::io)?;
    Ok(manifest)
}

// --- Walk: capture, verified copy and scan ------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Walk {
    /// Read a cleanly closed build store's `data/`, excluding runtime locks.
    Capture,
    /// Read an owner-private template, which holds no runtime state.
    Template,
}

fn runtime_lock(name: &str) -> bool {
    name == "LOCK" || name.ends_with(".lock")
}

fn unclean(name: &str) -> bool {
    matches!(name, "endpoint.json" | "migration.json" | "sql-server.info")
        || name.ends_with(".pid")
        || name.ends_with(".sock")
}

/// Sync a created directory's entries. Windows has no directory sync; its
/// file syncs and the no-replace publication rename carry durability there.
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(windows)]
    let _ = path;
    #[cfg(test)]
    hooks::synced(path);
    Ok(())
}

/// What a walk does with each entry as it reads it.
type Visit<'a> = &'a mut dyn FnMut(&Entry, Option<&[u8]>) -> Result<(), CreationFailure>;

/// Walk `source` in `mode`, copying into `destination` when given (every
/// file and directory synced), and calling `visit` with each entry, and with
/// each file's bytes when `keep` is set.
fn walk(
    source: &Path,
    mode: Walk,
    destination: Option<&Directory>,
    keep: bool,
    visit: Visit<'_>,
) -> Result<Vec<Entry>, CreationFailure> {
    let mut entries = Vec::new();
    let privacy = match mode {
        Walk::Capture => Privacy::Inherited,
        Walk::Template => Privacy::OwnerOnly,
    };
    walk_into(
        source,
        privacy,
        mode,
        destination,
        keep,
        &mut Vec::new(),
        &mut entries,
        visit,
    )
    .map_err(|failure| failure.context(format!("read store template tree {}", source.display())))?;
    Ok(entries)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one recursive walk shares its mode, destination and visitor"
)]
fn walk_into(
    source: &Path,
    privacy: Privacy,
    mode: Walk,
    destination: Option<&Directory>,
    keep: bool,
    path: &mut Vec<String>,
    entries: &mut Vec<Entry>,
    visit: Visit<'_>,
) -> Result<(), CreationFailure> {
    if path.len() >= MAX_DEPTH {
        return Err(refusal(mode, "the tree is nested too deeply".into()));
    }
    let directory = files::open_directory(source, privacy, NameRetention::Movable)
        .map_err(CreationFailure::Io)?;
    let mut names = fs::read_dir(source)
        .map_err(CreationFailure::io)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(CreationFailure::io)?;
    names.sort();
    for name in names {
        let Some(text) = name.to_str().map(str::to_owned) else {
            return Err(refusal(mode, format!("non-UTF-8 entry {name:?}")));
        };
        let object = source.join(&name);
        let kind = fs::symlink_metadata(&object)
            .map_err(CreationFailure::io)?
            .file_type();
        if kind.is_symlink() {
            return Err(refusal(mode, format!("refuses link {}", object.display())));
        }
        if unclean(&text) {
            return Err(refusal(
                mode,
                format!("{} was not cleanly closed", object.display()),
            ));
        }
        if runtime_lock(&text) {
            if mode == Walk::Capture && !kind.is_dir() {
                continue;
            }
            return Err(refusal(
                mode,
                format!("holds runtime lock state {}", object.display()),
            ));
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(refusal(mode, "the tree has too many entries".into()));
        }
        path.push(text);
        if kind.is_dir() {
            let child = destination
                .map(|parent| parent.create_private_directory(&name))
                .transpose()
                .map_err(CreationFailure::io)?;
            let entry = Entry::Directory { path: path.clone() };
            visit(&entry, None)?;
            entries.push(entry);
            let child_privacy = match mode {
                Walk::Capture => Privacy::Inherited,
                Walk::Template => Privacy::OwnerOnly,
            };
            walk_into(
                &object,
                child_privacy,
                mode,
                child.as_ref(),
                keep,
                path,
                entries,
                visit,
            )?;
            if let Some(child) = child {
                sync_directory(child.path()).map_err(CreationFailure::io)?;
            }
        } else if kind.is_file() {
            // Checked opens refuse links, including extra hard links.
            let mut input = directory.read(&name).map_err(|error| {
                if link_refusal(&error) {
                    refusal(mode, format!("{}: {error}", object.display()))
                } else {
                    CreationFailure::io(error)
                }
            })?;
            let mut output = destination
                .map(|parent| parent.create_new(&name))
                .transpose()
                .map_err(CreationFailure::io)?;
            #[cfg(test)]
            hooks::read_fault(&object)?;
            let (bytes, sha256, kept) = copy_hashing(mode, &mut input, output.as_mut(), keep)?;
            directory
                .verify(&name, &input)
                .map_err(CreationFailure::io)?;
            if let Some(output) = output {
                output.sync_all().map_err(CreationFailure::io)?;
                #[cfg(test)]
                if let Some(parent) = destination {
                    hooks::synced(&parent.path().join(&name));
                }
            }
            let entry = Entry::File {
                path: path.clone(),
                bytes,
                sha256,
            };
            visit(&entry, kept.as_deref())?;
            entries.push(entry);
        } else {
            return Err(refusal(
                mode,
                format!("refuses non-regular object {}", object.display()),
            ));
        }
        path.pop();
    }
    directory.revalidate().map_err(CreationFailure::io)?;
    Ok(())
}

fn copy_hashing(
    mode: Walk,
    input: &mut File,
    mut output: Option<&mut File>,
    keep: bool,
) -> Result<(u64, String, Option<Vec<u8>>), CreationFailure> {
    let mut limited = input.take(FILE_LIMIT + 1);
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut kept = keep.then(Vec::new);
    let mut buffer = [0; 65536];
    loop {
        let read = limited.read(&mut buffer).map_err(CreationFailure::io)?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > FILE_LIMIT {
            return Err(refusal(mode, "a file exceeds the size limit".into()));
        }
        hash.update(&buffer[..read]);
        if let Some(output) = output.as_deref_mut() {
            output
                .write_all(&buffer[..read])
                .map_err(CreationFailure::io)?;
        }
        if let Some(kept) = kept.as_mut() {
            kept.extend_from_slice(&buffer[..read]);
        }
    }
    Ok((count, hex(&hash.finalize()), kept))
}

/// Copy the judged template's `data/` into `stage/data`, verifying every
/// file's size and SHA-256 against the manifest while it writes, and the
/// entry set once it is complete. Links, extra hard links and objects of
/// another kind are refused. Every file and directory is synced. A verdict
/// or an I/O error leaves the partial copy where it is, for the caller to
/// preserve.
pub(crate) fn copy_into(judged: &Judged, stage: &Directory) -> Result<(), CreationFailure> {
    let data = stage
        .create_private_directory(OsStr::new(DATA))
        .map_err(CreationFailure::io)?;
    let mut expected = judged.manifest.entries.iter();
    let copied = walk(
        &judged.path.join(DATA),
        Walk::Template,
        Some(&data),
        false,
        &mut |entry, _| match expected.next() {
            Some(listed) if listed == entry => Ok(()),
            Some(listed) => Err(CreationFailure::verdict(format!(
                "{} differs from its manifest entry {}",
                entry.path(),
                listed.path()
            ))),
            None => Err(CreationFailure::verdict(format!(
                "{} is not in its manifest",
                entry.path()
            ))),
        },
    )?;
    if copied.len() != judged.manifest.entries.len() {
        return Err(CreationFailure::verdict(format!(
            "the template holds {} entries, its manifest {}",
            copied.len(),
            judged.manifest.entries.len()
        )));
    }
    sync_directory(data.path()).map_err(CreationFailure::io)?;
    sync_directory(stage.path()).map_err(CreationFailure::io)
}

/// Verify a whole template directory against its manifest without copying.
fn verify(template: &Directory, key: &str) -> Result<Manifest, CreationFailure> {
    let manifest = judge(template, key)?;
    let found = walk(
        &template.path().join(DATA),
        Walk::Template,
        None,
        false,
        &mut |_, _| Ok(()),
    )?;
    if found != manifest.entries {
        return Err(CreationFailure::verdict(
            "the template does not match its manifest",
        ));
    }
    Ok(manifest)
}

// --- Byte scan --------------------------------------------------------------------------

/// A labelled byte pattern the captured tree must not contain.
#[derive(Clone, Debug)]
pub(crate) struct Needle {
    pub(crate) label: String,
    bytes: Vec<u8>,
}

/// The UTF-8 and UTF-16LE spellings of `text`, with both separators and
/// without a Windows verbatim prefix.
fn spellings(label: &str, text: &str, needles: &mut Vec<Needle>) {
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let mut forms = vec![text.to_owned()];
    for variant in [text.replace('\\', "/"), text.replace('/', "\\")] {
        if !forms.contains(&variant) {
            forms.push(variant);
        }
    }
    for form in forms {
        needles.push(Needle {
            label: format!("{label} (UTF-16LE)"),
            bytes: form.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        });
        needles.push(Needle {
            label: format!("{label} (UTF-8)"),
            bytes: form.into_bytes(),
        });
    }
}

/// The build store's paths, both build secrets (whole and in 16-character
/// fragments) and the host names, each in every spelling.
fn leak_needles(store: &[PathBuf], secrets: &[String; 2], hostnames: &[String]) -> Vec<Needle> {
    let mut needles = Vec::new();
    for path in store {
        spellings(
            "the build store path",
            &path.to_string_lossy(),
            &mut needles,
        );
    }
    for (index, secret) in secrets.iter().enumerate() {
        let label = if index == 0 {
            "the root secret"
        } else {
            "the reader secret"
        };
        spellings(label, secret, &mut needles);
        for fragment in secret.as_bytes().chunks(16) {
            spellings(
                &format!("a 16-character fragment of {label}"),
                &String::from_utf8_lossy(fragment),
                &mut needles,
            );
        }
    }
    for hostname in hostnames {
        if hostname.len() >= HOSTNAME_NEEDLE_MIN {
            spellings("the host name", hostname, &mut needles);
        }
    }
    needles
}

/// Every needle found in the template tree `data`, as `(file, label)`.
fn scan(data: &Path, needles: &[Needle]) -> Result<Vec<(String, String)>, CreationFailure> {
    let mut hits = Vec::new();
    walk(data, Walk::Template, None, true, &mut |entry, bytes| {
        if let Some(bytes) = bytes {
            for needle in needles {
                if !needle.bytes.is_empty()
                    && bytes
                        .windows(needle.bytes.len())
                        .any(|window| window == needle.bytes.as_slice())
                {
                    hits.push((entry.path(), needle.label.clone()));
                }
            }
        }
        Ok(())
    })?;
    Ok(hits)
}

/// The host names an engine may write: the engine's own `@@hostname` and,
/// on Windows, `COMPUTERNAME`.
fn hostnames(engine_hostname: &str) -> Vec<String> {
    let mut names = vec![engine_hostname.to_owned()];
    if cfg!(windows)
        && let Some(name) = std::env::var_os("COMPUTERNAME")
    {
        names.push(name.to_string_lossy().into_owned());
    }
    names.retain(|name| !name.is_empty());
    names.sort();
    names.dedup();
    names
}

// --- Build ------------------------------------------------------------------------------

/// The engine a template is built with.
#[derive(Clone, Debug)]
pub(crate) struct Engine {
    pub(crate) binary: PathBuf,
    pub(crate) supervisor: PathBuf,
    pub(crate) timeout: Duration,
}

/// Abandoned build and capture stages of one key a sweep removed or left.
#[derive(Debug, Default)]
pub(crate) struct Swept {
    pub(crate) removed: Vec<String>,
    pub(crate) left: Vec<SweepLeft>,
}

/// An abandoned entry a sweep left for a later holder, and why.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the template cache's own tests read a build's report"
    )
)]
#[derive(Debug)]
pub(crate) struct SweepLeft {
    pub(crate) name: String,
    pub(crate) error: String,
}

/// What one build did.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the template cache's own tests read a build's report"
    )
)]
#[derive(Debug)]
pub(crate) struct BuildReport {
    pub(crate) swept: Swept,
    /// The build store, removed once captured (or left for a sweep).
    pub(crate) build_store: PathBuf,
    /// Whether the verified capture stage was published as `<key>/`.
    pub(crate) published: bool,
    /// The captured `data/`: files and their total bytes.
    pub(crate) files: usize,
    pub(crate) bytes: u64,
    /// The engine's `@@hostname`, scanned for.
    pub(crate) hostname: String,
    /// The needles the byte scan looked for, for the build's own tests.
    #[cfg(test)]
    pub(crate) needles: Vec<Needle>,
}

/// Remove, without waiting, this key's abandoned `.build-*` and `.stage-*`
/// entries. Only an exclusive holder calls it. A build store whose lifecycle
/// lease is still held after one attempt at the smallest valid wait, and any
/// entry whose removal fails, is left for a later holder. Never an error.
async fn sweep(root: &Directory, key: &str, lifecycle_root: Option<&Path>) -> Swept {
    let mut swept = Swept::default();
    let build_prefix = format!(".build-{key}-");
    let stage_prefix = format!(".stage-{key}-");
    let names = match fs::read_dir(root.path()) {
        Ok(entries) => entries
            .take(ROOT_ENTRY_LIMIT)
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        Err(error) => {
            tracing::warn!(error = %error, "store template sweep could not read its root");
            return swept;
        }
    };
    for name in names {
        let path = root.path().join(&name);
        let result = if name.starts_with(&build_prefix) {
            sweep_build(&path, lifecycle_root).await
        } else if name.starts_with(&stage_prefix) {
            files::directory(&path).and_then(|stage| Ok(stage.remove_tree()?))
        } else {
            continue;
        };
        match result {
            Ok(()) => swept.removed.push(name),
            Err(error) => {
                let error = format!("{error:#}");
                tracing::warn!(
                    entry = %name,
                    error = %error,
                    "store template sweep left an abandoned entry for a later holder"
                );
                swept.left.push(SweepLeft { name, error });
            }
        }
    }
    swept
}

/// Under an exclusive key lock already held for a quarantine: sweep this
/// key's abandoned entries as a build would, so a build store left after
/// its template was published does not outlive the next exclusive holder.
/// Best-effort: an unusable lifecycle root skips the sweep with a warning.
async fn sweep_held(root: &Directory, key: &str) -> Swept {
    match build_lifecycle_root(root) {
        Ok(lifecycle_root) => sweep(root, key, lifecycle_root.as_deref()).await,
        Err(error) => {
            tracing::warn!(
                error = %format!("{error:#}"),
                "store template sweep skipped: its lifecycle root is unusable"
            );
            Swept::default()
        }
    }
}

async fn sweep_build(path: &Path, lifecycle_root: Option<&Path>) -> Result<()> {
    let store = path.join(BUILD_STORE);
    match fs::symlink_metadata(&store) {
        Ok(metadata) if metadata.is_dir() => {
            // The lease proves no engine is live there.
            let lease = Server::quiescence_at(&store, lifecycle_root, SWEEP_QUIESCENCE).await?;
            remove_build_store(lease).context("remove the abandoned store template build store")?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    files::directory(path)?
        .remove_tree()
        .context("remove the emptied store template build directory")?;
    Ok(())
}

/// Remove a reaped build store through its own lifecycle lease, which this
/// consumes. The lease's handle on the store is the one the removal
/// consumes: a second handle opened beside it would keep the store's name
/// delete-pending on Windows until the lease dropped, so its parent could
/// not be removed.
fn remove_build_store(lease: crate::server::LifecycleLease) -> Result<()> {
    lease.remove_tree()
}

/// Build, capture, verify and publish this key's template. The caller holds
/// the exclusive key lock in `lock`; it becomes the build engine's reap
/// guard and is returned only after that engine was reaped, so it is
/// returned on success and on every failure after the engine start. An
/// engine start that fails keeps it until its reap.
async fn build(
    root: &Directory,
    key: &str,
    lock: File,
    engine: &Engine,
) -> Result<(File, Judged, BuildReport), (Option<File>, CreationFailure)> {
    let lifecycle_root = match build_lifecycle_root(root) {
        Ok(lifecycle_root) => lifecycle_root,
        Err(error) => return Err((Some(lock), CreationFailure::Io(error))),
    };
    let swept = sweep(root, key, lifecycle_root.as_deref()).await;
    let prepared = (|| -> Result<PathBuf> {
        let build = root.create_private_directory(OsStr::new(&transient_name("build", key)))?;
        let store = build.create_private_directory(OsStr::new(BUILD_STORE))?;
        crate::server::write_template_build_identity(store.path(), key)?;
        Ok(store.path().to_owned())
    })();
    let store = match prepared {
        Ok(store) => store,
        Err(error) => return Err((Some(lock), CreationFailure::Io(error))),
    };
    let make_options = |directory: PathBuf, read_only: bool| ServerOptions {
        binary: engine.binary.clone(),
        directory,
        project_scope: TEMPLATE_SCOPE.to_owned(),
        supervisor: engine.supervisor.clone(),
        timeout: engine.timeout,
        read_only,
        retained: None,
        lifecycle_root: lifecycle_root.clone(),
    };
    let worker = stage_worker::StageWorker {
        make_options: &make_options,
        stage: &store,
        parent: root.path(),
        lifecycle_root: lifecycle_root.as_deref(),
        timeout: engine.timeout,
        project_scope: TEMPLATE_SCOPE,
        legacy: None,
        #[cfg(test)]
        hooks: stage_worker::StageHooks::default(),
    };
    let mut progress = ProgressReporter::silent();
    let (lock, hostname) = match worker.build_template(lock, &mut progress).await {
        Ok(built) => built,
        Err(error) => {
            return Err((
                None,
                CreationFailure::classify(error.context("build the memory store template")),
            ));
        }
    };
    match capture(root, key, &store, lifecycle_root.as_deref(), &hostname).await {
        Ok((stage, manifest, needles)) => {
            let (files, bytes) = manifest
                .entries
                .iter()
                .fold((0, 0), |(files, bytes), entry| match entry {
                    Entry::File { bytes: size, .. } => (files + 1, bytes + size),
                    Entry::Directory { .. } => (files, bytes),
                });
            #[cfg(not(test))]
            let _ = needles;
            let (template, published) = publish(root, key, stage, manifest).await;
            let report = BuildReport {
                swept,
                build_store: store,
                published,
                files,
                bytes,
                hostname,
                #[cfg(test)]
                needles,
            };
            match template {
                Ok(template) => Ok((lock, template, report)),
                Err(failure) => Err((Some(lock), failure)),
            }
        }
        Err(failure) => Err((Some(lock), failure)),
    }
}

/// Capture the reaped build store's `data/` into a new capture stage, scan
/// it, describe it and verify it, then remove the build store. Returns the
/// verified stage and its manifest. A failure removes the capture stage at
/// once (best-effort, so a byte-scan hit does not stay in the cache) and
/// leaves the build store for a sweep.
async fn capture(
    root: &Directory,
    key: &str,
    store: &Path,
    lifecycle_root: Option<&Path>,
    hostname: &str,
) -> Result<(Directory, Manifest, Vec<Needle>), CreationFailure> {
    // Holding the lifecycle lease proves the supervisor reaped Dolt.
    let lease = Server::quiescence_at(store, lifecycle_root, SWEEP_QUIESCENCE)
        .await
        .map_err(|error| {
            CreationFailure::Engine(error.context("the store template build did not quiesce"))
        })?;
    let secrets = crate::server::template_build_secrets(store).map_err(CreationFailure::Io)?;
    let mut paths = vec![store.to_owned()];
    if let Ok(canonical) = fs::canonicalize(store) {
        paths.push(canonical);
    }
    let needles = leak_needles(&paths, &secrets, &hostnames(hostname));
    #[cfg(test)]
    hooks::before_capture(store).map_err(CreationFailure::Io)?;
    let stage = root
        .create_private_directory(OsStr::new(&transient_name("stage", key)))
        .map_err(CreationFailure::io)?;
    let manifest = match describe(root, key, store, &stage, &needles) {
        Ok(manifest) => manifest,
        Err(failure) => {
            discard_stage(stage);
            return Err(failure);
        }
    };
    // The template is complete without the build store; a removal failure
    // leaves it for a sweep. The store goes through the lease's own handle
    // (still the reap proof), and only then its emptied parent.
    let removed = remove_build_store(lease).and_then(|()| match store.parent() {
        Some(build) => Ok(files::directory(build)?.remove_tree()?),
        None => Ok(()),
    });
    if let Err(error) = removed {
        tracing::warn!(
            build = %store.display(),
            error = %format!("{error:#}"),
            "store template build store left for a later sweep"
        );
    }
    Ok((stage, manifest, needles))
}

/// Remove a capture stage that failed, best-effort: a failed removal leaves
/// it for the next exclusive holder's sweep.
fn discard_stage(stage: Directory) {
    let path = stage.path().to_owned();
    if let Err(error) = stage.remove_tree() {
        tracing::warn!(
            stage = %path.display(),
            error = %format!("{error:#}"),
            "a failed store template capture stage was left for a later sweep"
        );
    }
}

/// Copy the build store's `data/` into `stage`, scan it, write its manifest
/// and verify the stage against it.
fn describe(
    root: &Directory,
    key: &str,
    store: &Path,
    stage: &Directory,
    needles: &[Needle],
) -> Result<Manifest, CreationFailure> {
    let data = stage
        .create_private_directory(OsStr::new(DATA))
        .map_err(CreationFailure::io)?;
    let entries = walk(
        &store.join(DATA),
        Walk::Capture,
        Some(&data),
        false,
        &mut |_, _| Ok(()),
    )?;
    sync_directory(data.path()).map_err(CreationFailure::io)?;
    let hits = scan(data.path(), needles)?;
    if !hits.is_empty() {
        let shown = hits
            .iter()
            .map(|(file, label)| format!("{label} in {file}"))
            .collect::<Vec<_>>();
        return Err(CreationFailure::Engine(anyhow!(
            "the captured store template holds build-private bytes; refusing to publish: {}",
            shown.join("; ")
        )));
    }
    let manifest = Manifest {
        format: TEMPLATE_FORMAT,
        key: key.to_owned(),
        entries,
    };
    let written = (|| -> std::io::Result<()> {
        let mut file = stage.create_new(OsStr::new(MANIFEST))?;
        file.write_all(&serde_json::to_vec(&manifest)?)?;
        file.sync_all()?;
        #[cfg(test)]
        hooks::manifest_written();
        sync_directory(stage.path())?;
        sync_directory(root.path())
    })();
    written.map_err(CreationFailure::io)?;
    verify(stage, key).map_err(|failure| {
        failure.context("verify the captured store template before publication".into())
    })
}

/// Publish the verified capture stage as `<key>/` by a no-replace rename,
/// retried for a bounded time while the stage provably stayed in place. On
/// failure the verified stage is returned for its builder to copy from, and
/// left for the next exclusive holder's sweep.
async fn publish(
    root: &Directory,
    key: &str,
    stage: Directory,
    manifest: Manifest,
) -> (Result<Judged, CreationFailure>, bool) {
    let destination = root.path().join(key);
    let identity = stage.identity();
    let deadline = Instant::now() + PUBLISH_RETRY_LIMIT;
    loop {
        #[cfg(test)]
        let moved = match hooks::publish_fault() {
            Err(error) => Err(error),
            Ok(()) => files::move_directory(&stage, &destination),
        };
        #[cfg(not(test))]
        let moved = files::move_directory(&stage, &destination);
        match moved {
            Ok(published) => {
                return (
                    Ok(Judged {
                        path: destination,
                        identity: published.identity(),
                        manifest,
                    }),
                    true,
                );
            }
            Err(error) => {
                let unmoved = stage.revalidate().is_ok();
                let occupied = fs::symlink_metadata(&destination).is_ok();
                if unmoved && !occupied && Instant::now() < deadline {
                    tokio::time::sleep(PUBLISH_RETRY_SPACING).await;
                    continue;
                }
                tracing::warn!(
                    error = %format!("{error:#}"),
                    stage = %stage.path().display(),
                    "store template publication failed; its verified stage is kept for a later sweep"
                );
                if unmoved {
                    return (
                        Ok(Judged {
                            path: stage.path().to_owned(),
                            identity,
                            manifest,
                        }),
                        false,
                    );
                }
                return (
                    Err(CreationFailure::Io(
                        error.context("publish the memory store template"),
                    )),
                    false,
                );
            }
        }
    }
}

// --- Quarantine -------------------------------------------------------------------------

/// What a quarantine attempt did.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Quarantine {
    Moved(PathBuf),
    Skipped(String),
}

fn skipped(reason: String) -> Quarantine {
    tracing::warn!(reason = %reason, "store template quarantine skipped");
    Quarantine::Skipped(reason)
}

/// Quarantine the template judged as `judged` after a verdict, holding no
/// key lock: take the exclusive lock without waiting, then
/// [`quarantine_held`]. Best-effort.
async fn quarantine(root: &Directory, key: &str, judged: FileIdentity) -> Quarantine {
    #[cfg(test)]
    hooks::before_quarantine().await;
    match try_key_lock(root, key, Mode::Exclusive) {
        Ok(Some(lock)) => {
            let moved = quarantine_held(root, key, judged);
            sweep_held(root, key).await;
            drop(lock);
            moved
        }
        Ok(None) => skipped("the store template key lock is busy".into()),
        Err(error) => skipped(format!("{error:#}")),
    }
}

/// Under the exclusive key lock: move `<key>/` aside to a new
/// `.rejected-<key>-<uuid>` only if it still is the directory judged, after
/// removing an older quarantined directory of this key. Best-effort.
fn quarantine_held(root: &Directory, key: &str, judged: FileIdentity) -> Quarantine {
    let path = root.path().join(key);
    let template = match files::directory(&path) {
        Ok(template) => template,
        Err(error) => return skipped(format!("open the judged template: {error:#}")),
    };
    if template.identity() != judged {
        return skipped("the published template is no longer the one judged".into());
    }
    remove_rejected(root, key);
    // The one handle that was compared is the one moved: no second handle
    // can hold the source open across the rename.
    let target = root.path().join(transient_name("rejected", key));
    match files::move_directory(&template, &target) {
        Ok(_) => {
            tracing::warn!(
                template = %path.display(),
                quarantined = %target.display(),
                "store template quarantined after a verdict against its bytes"
            );
            Quarantine::Moved(target)
        }
        Err(error) => skipped(format!("move the judged template aside: {error:#}")),
    }
}

/// Remove this key's older quarantined directories, best-effort.
fn remove_rejected(root: &Directory, key: &str) {
    let prefix = format!(".rejected-{key}-");
    let Ok(entries) = fs::read_dir(root.path()) else {
        return;
    };
    for entry in entries
        .take(ROOT_ENTRY_LIMIT)
        .filter_map(|entry| entry.ok())
    {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(&prefix) {
            continue;
        }
        if let Err(error) =
            files::directory(&entry.path()).and_then(|rejected| Ok(rejected.remove_tree()?))
        {
            tracing::warn!(
                entry = %entry.path().display(),
                error = %format!("{error:#}"),
                "an older quarantined store template was left in place"
            );
        }
    }
}

// --- Entry points -------------------------------------------------------------------------

/// How long a key-lock holder may be waited for.
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "only test-fixture warm-up and prefetch wait for or ensure a template; an open uses create_in"
    )
)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Wait {
    /// Product openers: a busy lock means no template now.
    Never,
    /// Fixture warm-up and prefetch: poll until this instant.
    Until(Instant),
}

/// Why no template can be used now.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Unavailable {
    /// Another holder excludes the key lock: a build or a quarantine.
    Busy,
    /// The root or the key lock could not be opened, locked or verified.
    Lock(String),
}

/// What [`ensure_in`] found or did.
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "only test-fixture warm-up and prefetch wait for or ensure a template; an open uses create_in"
    )
)]
#[derive(Debug)]
pub(crate) enum Ensured {
    /// A structurally valid template was already published.
    Published,
    /// This call built the template.
    Built(BuildReport),
    /// [`Wait::Never`] only.
    Unavailable(Unavailable),
}

/// A lock or root error: "no template" with a warning for a product caller,
/// fatal and named for a waiting one.
fn lock_unavailable<T>(
    wait: Wait,
    error: anyhow::Error,
    unavailable: impl FnOnce(Unavailable) -> T,
) -> Result<T, CreationFailure> {
    match wait {
        Wait::Never => {
            tracing::warn!(
                error = %format!("{error:#}"),
                "store template lock unavailable; creating without a template"
            );
            Ok(unavailable(Unavailable::Lock(format!("{error:#}"))))
        }
        Wait::Until(_) => Err(CreationFailure::Io(
            error.context("store template lock failed during warm-up"),
        )),
    }
}

/// Whether a busy key lock ends the call ([`Wait::Never`]) or is polled.
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "only test-fixture warm-up and prefetch wait for or ensure a template; an open uses create_in"
    )
)]
async fn busy(wait: Wait, root: &Directory, key: &str) -> Result<bool, CreationFailure> {
    match wait {
        Wait::Never => Ok(true),
        Wait::Until(deadline) => {
            if Instant::now() >= deadline {
                return Err(CreationFailure::Io(anyhow!(
                    "store template lock {} stayed held until the warm-up bound; another process may \
                     still be building the template",
                    root.path().join(lock_name(key)).display()
                )));
            }
            tokio::time::sleep(LOCK_POLL).await;
            Ok(false)
        }
    }
}

/// Make sure this build's template is published under `root`: check its
/// structure under the shared lock, or build it under the exclusive lock.
/// With [`Wait::Never`] a busy or failing lock returns
/// [`Ensured::Unavailable`]; with [`Wait::Until`] a busy lock is polled and a
/// lock error is fatal. A structural verdict quarantines the judged template
/// under the exclusive lock (identity-bound, best-effort). With
/// [`Wait::Never`] the verdict is then returned, so a caller never pays a
/// rebuild on top of a damaged template; with [`Wait::Until`] (warm-up and
/// prefetch) the quarantine is followed by a rebuild. A product opener uses
/// [`create_in`], which never rebuilds after a verdict either.
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "only test-fixture warm-up and prefetch wait for or ensure a template; an open uses create_in"
    )
)]
pub(crate) async fn ensure_in(
    root: &Path,
    engine: &Engine,
    wait: Wait,
) -> Result<Ensured, CreationFailure> {
    let key = compiled_key();
    let root = match open_root(root) {
        Ok(root) => root,
        Err(error) => return lock_unavailable(wait, error, Ensured::Unavailable),
    };
    loop {
        match try_key_lock(&root, key, Mode::Shared) {
            Err(error) => return lock_unavailable(wait, error, Ensured::Unavailable),
            Ok(None) => {
                if busy(wait, &root, key).await? {
                    return Ok(Ensured::Unavailable(Unavailable::Busy));
                }
                continue;
            }
            Ok(Some(shared)) => {
                let inspected = inspect(&root, key);
                drop(shared);
                match inspected? {
                    Inspection::Valid(_) => return Ok(Ensured::Published),
                    // The exclusive holder re-judges and quarantines.
                    Inspection::Absent | Inspection::Condemned { .. } => {}
                }
            }
        }
        let lock = match try_key_lock(&root, key, Mode::Exclusive) {
            Err(error) => return lock_unavailable(wait, error, Ensured::Unavailable),
            Ok(None) => {
                if busy(wait, &root, key).await? {
                    return Ok(Ensured::Unavailable(Unavailable::Busy));
                }
                continue;
            }
            Ok(Some(lock)) => lock,
        };
        match inspect(&root, key)? {
            Inspection::Valid(_) => return Ok(Ensured::Published),
            Inspection::Absent => {}
            Inspection::Condemned { identity, failure } => {
                let moved = identity.map(|identity| quarantine_held(&root, key, identity));
                if matches!(wait, Wait::Never) {
                    // A build would sweep; this holder builds nothing.
                    sweep_held(&root, key).await;
                    drop(lock);
                    return Err(failure.context(
                        "the published store template failed its structural check".into(),
                    ));
                }
                if !matches!(moved, Some(Quarantine::Moved(_))) {
                    return Err(failure.context(
                        "the published store template failed its structural check and could not \
                         be quarantined"
                            .into(),
                    ));
                }
            }
        }
        // Boxed: a build nests a complete engine start and the chain.
        return match Box::pin(build(&root, key, lock, engine)).await {
            Ok((lock, _, report)) => {
                drop(lock);
                Ok(Ensured::Built(report))
            }
            Err((lock, failure)) => {
                drop(lock);
                Err(failure)
            }
        };
    }
}

/// How [`create_in`] ended.
#[derive(Debug)]
pub(crate) enum Created {
    /// The stage holds a verified copy of this key's template. `judged` is
    /// the identity of the published template it was copied from, which a
    /// later verdict on the copy's own engine quarantines
    /// ([`quarantine_after_adoption`]); `None` when the copy came from a
    /// build's verified but unpublished stage.
    Copied {
        built: bool,
        published: bool,
        judged: Option<FileIdentity>,
    },
    /// No template now; the caller creates the store cold.
    Unavailable(Unavailable),
}

/// How [`create_in`] failed, by the phase that failed. The caller's action
/// depends on the phase, not on the kind of evidence.
#[derive(Debug)]
pub(crate) enum CreateError {
    /// Using a template this call did not build failed: its structural check
    /// or the copy from it. A verdict among them already quarantined the
    /// judged template. The caller creates the store cold.
    Use(CreationFailure),
    /// This call's template build failed: its engine, its own assertions (a
    /// shape verdict there is against the build's unpublished bytes), its
    /// capture, or its publication with no verified stage to copy from. The
    /// chain may already have run, so the caller fails the open with this
    /// error and runs no other creation in it, as it would not rebuild after
    /// a verdict.
    Build(CreationFailure),
    /// This call's template build succeeded, and this project's copy from the
    /// template or verified stage it produced failed. A verdict on that copy
    /// already quarantined the template this call published. The chain ran
    /// in this call, so the caller fails the open as it would for
    /// [`Self::Build`], but reports the copy, not the build, as what failed.
    BuiltCopy(CreationFailure),
}

#[cfg(test)]
impl CreateError {
    pub(crate) fn failure(&self) -> &CreationFailure {
        match self {
            Self::Use(failure) | Self::Build(failure) | Self::BuiltCopy(failure) => failure,
        }
    }

    pub(crate) fn is_verdict(&self) -> bool {
        self.failure().is_verdict()
    }
}

impl From<CreationFailure> for CreateError {
    fn from(failure: CreationFailure) -> Self {
        Self::Use(failure)
    }
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use(failure) => write!(formatter, "{failure}"),
            Self::Build(failure) => {
                write!(formatter, "the store template build failed: {failure}")
            }
            Self::BuiltCopy(failure) => write!(
                formatter,
                "the copy from the store template this open built failed: {failure}"
            ),
        }
    }
}

impl std::error::Error for CreateError {}

/// [`copy_into`] on a blocking thread, which also holds the key lock `lock`
/// for as long as the copy writes: a cancelled caller cannot release the
/// lock under a copy in flight. The lock comes back with the outcome; a copy
/// that panicked returns none, and its lock was released as the copy
/// unwound. The thread writes through its own handle on `stage`, opened
/// again and bound to `stage`'s identity.
async fn copy_blocking(
    judged: Judged,
    stage: &Directory,
    lock: File,
) -> (Option<File>, Result<(), CreationFailure>) {
    let destination = match files::directory(stage.path()) {
        Ok(destination) if destination.identity() == stage.identity() => destination,
        Ok(_) => {
            return (
                Some(lock),
                Err(CreationFailure::Io(anyhow!(
                    "the store stage {} was replaced before its template copy",
                    stage.path().display()
                ))),
            );
        }
        Err(error) => return (Some(lock), Err(CreationFailure::Io(error))),
    };
    #[cfg(test)]
    let scoped = hooks::captured();
    let copied = tokio::task::spawn_blocking(move || {
        let copy = || copy_into(&judged, &destination);
        #[cfg(test)]
        let outcome = match scoped {
            Some(scoped) => hooks::HOOKS.sync_scope(scoped, copy),
            None => copy(),
        };
        #[cfg(not(test))]
        let outcome = copy();
        (lock, outcome)
    })
    .await;
    match copied {
        Ok((lock, outcome)) => (Some(lock), outcome),
        Err(error) => (
            None,
            Err(CreationFailure::Io(anyhow!(
                "the store template copy stopped: {error}"
            ))),
        ),
    }
}

/// Copy this build's template into `stage/data`, building it first when
/// none is published and the exclusive key lock is free. It never waits for
/// a lock. A verdict against a published template quarantines that template
/// (identity-bound, best-effort) and is returned; every other failure is
/// returned and leaves every template untouched. A failure of the build is
/// returned as [`CreateError::Build`], a failure of the copy from the
/// template or verified stage it built as [`CreateError::BuiltCopy`], every
/// other one as [`CreateError::Use`]. On any
/// failure the partial copy stays in `stage`
/// for the caller to preserve. Each copy runs on a blocking thread that
/// holds the key lock until it returns.
pub(crate) async fn create_in(
    root: &Path,
    engine: &Engine,
    stage: &Directory,
) -> Result<Created, CreateError> {
    let key = compiled_key();
    let directory = match open_root(root) {
        Ok(directory) => directory,
        Err(error) => {
            return lock_unavailable(Wait::Never, error, Created::Unavailable).map_err(Into::into);
        }
    };
    let root = &directory;
    match try_key_lock(root, key, Mode::Shared) {
        Err(error) => {
            return lock_unavailable(Wait::Never, error, Created::Unavailable).map_err(Into::into);
        }
        Ok(None) => return Ok(Created::Unavailable(Unavailable::Busy)),
        Ok(Some(shared)) => {
            let judged = match inspect(root, key) {
                Ok(Inspection::Valid(judged)) => Some(Ok(judged)),
                Ok(Inspection::Absent) => None,
                Ok(Inspection::Condemned { identity, failure }) => Some(Err((identity, failure))),
                Err(failure) => return Err(failure.into()),
            };
            match judged {
                None => drop(shared),
                Some(Ok(judged)) => {
                    let identity = judged.identity;
                    let (shared, copied) = copy_blocking(judged, stage, shared).await;
                    drop(shared);
                    return match copied {
                        Ok(()) => Ok(Created::Copied {
                            built: false,
                            published: true,
                            judged: Some(identity),
                        }),
                        Err(failure) => {
                            condemn(root, key, Some(identity), &failure, stage).await;
                            Err(failure.into())
                        }
                    };
                }
                Some(Err((identity, failure))) => {
                    drop(shared);
                    condemn(root, key, identity, &failure, stage).await;
                    return Err(failure.into());
                }
            }
        }
    }
    let lock = match try_key_lock(root, key, Mode::Exclusive) {
        Err(error) => {
            return lock_unavailable(Wait::Never, error, Created::Unavailable).map_err(Into::into);
        }
        Ok(None) => return Ok(Created::Unavailable(Unavailable::Busy)),
        Ok(Some(lock)) => lock,
    };
    match inspect(root, key)? {
        Inspection::Valid(judged) => {
            let identity = judged.identity;
            let (lock, copied) = copy_blocking(judged, stage, lock).await;
            if let Err(failure) = &copied
                && failure.is_verdict()
            {
                // A copy that stopped without its lock leaves the quarantine
                // to the next copier, which judges the template again.
                if lock.is_some() {
                    let moved = quarantine_held(root, key, identity);
                    account(root.path(), stage.path(), &moved);
                    sweep_held(root, key).await;
                }
            }
            drop(lock);
            Ok(copied.map(|()| Created::Copied {
                built: false,
                published: true,
                judged: Some(identity),
            })?)
        }
        Inspection::Condemned { identity, failure } => {
            if let Some(identity) = identity {
                let moved = quarantine_held(root, key, identity);
                account(root.path(), stage.path(), &moved);
            }
            sweep_held(root, key).await;
            drop(lock);
            Err(failure.into())
        }
        Inspection::Absent => match Box::pin(build(root, key, lock, engine)).await {
            Ok((lock, template, report)) => {
                #[cfg(any(test, feature = "test-support"))]
                if is_shared_root(root.path()) {
                    crate::test_support::engine_ledger::template_event(
                        stage.path(),
                        crate::test_support::engine_ledger::TemplateEvent::Built,
                    );
                }
                // Copied from the published template, or from the verified
                // stage when publication failed; the chain is paid once.
                let judged = report.published.then_some(template.identity);
                let (lock, copied) = copy_blocking(template, stage, lock).await;
                match copied {
                    Ok(()) => {
                        drop(lock);
                        Ok(Created::Copied {
                            built: true,
                            published: report.published,
                            judged,
                        })
                    }
                    // The chain already ran in this open: a cold retry would
                    // run it again, so the copy's failure fails the open as
                    // the build's would. A verdict against the template this
                    // open published moves it aside under the lock still
                    // held; an unpublished verified stage is left for the
                    // next exclusive holder's sweep.
                    Err(failure) => {
                        if failure.is_verdict()
                            && let (Some(judged), Some(_)) = (judged, &lock)
                        {
                            let moved = quarantine_held(root, key, judged);
                            account(root.path(), stage.path(), &moved);
                            sweep_held(root, key).await;
                        }
                        drop(lock);
                        Err(CreateError::BuiltCopy(failure))
                    }
                }
            }
            Err((lock, failure)) => {
                drop(lock);
                Err(CreateError::Build(failure))
            }
        },
    }
}

/// After a verdict reached under the shared lock: quarantine the judged
/// template, bound to its identity. Other failures touch nothing.
async fn condemn(
    root: &Directory,
    key: &str,
    identity: Option<FileIdentity>,
    failure: &CreationFailure,
    stage: &Directory,
) {
    if !failure.is_verdict() {
        return;
    }
    let moved = match identity {
        Some(identity) => quarantine(root, key, identity).await,
        None => skipped("the judged template is not a directory".into()),
    };
    account(root.path(), stage.path(), &moved);
}

/// After a verdict on a copy's own engine (its adoption or the template
/// shape): quarantine the published template the copy came from, bound to
/// the identity it had when it was judged, holding no key lock until then.
/// Best-effort, like every quarantine: it never fails the caller, whose own
/// error stands.
pub(crate) async fn quarantine_after_adoption(
    root: &Path,
    judged: FileIdentity,
    stage: &Path,
) -> Quarantine {
    let directory = match open_root(root) {
        Ok(directory) => directory,
        Err(error) => return skipped(format!("{error:#}")),
    };
    let moved = quarantine(&directory, compiled_key(), judged).await;
    account(directory.path(), stage, &moved);
    moved
}

/// Test support: record a quarantine under the shared test root against the
/// fixture whose creation caused it. Under `cfg(test)` every outcome is also
/// recorded by root (`hooks::quarantines`).
fn account(root: &Path, stage: &Path, moved: &Quarantine) {
    #[cfg(test)]
    hooks::quarantined(root, moved);
    #[cfg(any(test, feature = "test-support"))]
    if matches!(moved, Quarantine::Moved(_)) && is_shared_root(root) {
        crate::test_support::engine_ledger::template_event(
            stage,
            crate::test_support::engine_ledger::TemplateEvent::Quarantined,
        );
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = (root, stage, moved);
}

#[cfg(any(test, feature = "test-support"))]
fn is_shared_root(root: &Path) -> bool {
    crate::test_support::is_shared_template_root(root)
}

#[cfg(test)]
pub(crate) mod hooks;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod open_tests;
