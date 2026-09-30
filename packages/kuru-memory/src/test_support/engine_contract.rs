//! Test-only support for the engine contract tests in
//! `store/engine_contract_tests.rs` (cospec change `memory-engine-contract-tests`).
//!
//! Three helpers, each exercised by those tests and by the unit tests below:
//!
//! - [`copy_data_tree`] copies a stopped store's `data/` tree through checked
//!   directory handles. It applies the capture rules of the test template
//!   (`template.rs`), restricted to `data/`: links and non-regular objects are
//!   refused, engine lock files (`LOCK`, `*.lock`) are skipped, and a Dolt
//!   server info, PID or socket file means the store was not stopped and
//!   refuses the copy. The source is read with inherited access, so a capture
//!   downloaded by CI (ordinary file modes) is readable; every destination
//!   directory and file is created fresh and owner-only.
//! - [`scan`] byte-scans every file below a directory for labelled needles.
//! - The cross-OS capture format ([`Capture`], [`write_capture`],
//!   [`read_capture_record`], [`read_capture_tree`], [`cross_os_plan`]): a directory holding
//!   `capture.json` and a `data/` tree. The record holds only relative path
//!   components, sizes and SHA-256 digests, the producing OS and
//!   architecture, the engine version, the project scope and instance, and
//!   the two movable refs' heads; never an absolute path, a host name or a
//!   secret. A consumer copies and verifies the whole tree against the record.
use crate::files;
use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 16;
const FILE_LIMIT: u64 = 256 * 1024 * 1024;
const RECORD_LIMIT: u64 = 4 * 1024 * 1024;

/// Bump when the capture layout or record fields change.
pub(crate) const CAPTURE_FORMAT: u32 = 1;
pub(crate) const CAPTURE_RECORD: &str = "capture.json";
pub(crate) const CAPTURE_DATA: &str = "data";
/// A capture directory produced on another operating system.
pub(crate) const CROSS_OS_CAPTURE: &str = "KURU_ENGINE_CONTRACT_CROSS_OS_CAPTURE";
/// `1` makes a not-run cross-OS result a failure (for a CI consuming job).
pub(crate) const REQUIRE_CROSS_OS: &str = "KURU_ENGINE_CONTRACT_REQUIRE_CROSS_OS";

/// One object below a copied or scanned root, by relative path components.
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
    pub(crate) fn path(&self) -> String {
        match self {
            Self::Directory { path } | Self::File { path, .. } => path.join("/"),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn runtime_lock(name: &str) -> bool {
    name == "LOCK" || name.ends_with(".lock")
}

fn unclean(name: &str) -> bool {
    name == "sql-server.info" || name.ends_with(".pid") || name.ends_with(".sock")
}

/// Walk `source`, calling `visit` with every regular file's relative path and
/// bytes, and copy into `destination` when one is given.
fn walk(
    source: &Path,
    destination: Option<&Directory>,
    visit: &mut dyn FnMut(&str, &[u8]) -> Result<()>,
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    walk_into(source, destination, &mut Vec::new(), &mut entries, visit)
        .with_context(|| format!("walk engine contract tree {}", source.display()))?;
    Ok(entries)
}

fn walk_into(
    source: &Path,
    destination: Option<&Directory>,
    path: &mut Vec<String>,
    entries: &mut Vec<Entry>,
    visit: &mut dyn FnMut(&str, &[u8]) -> Result<()>,
) -> Result<()> {
    ensure!(
        path.len() < MAX_DEPTH,
        "engine contract tree is nested too deeply"
    );
    let directory = files::open_directory(source, Privacy::Inherited, NameRetention::Movable)?;
    let mut names = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    for name in names {
        let text = name
            .to_str()
            .with_context(|| format!("non-UTF-8 engine contract entry {name:?}"))?
            .to_owned();
        let object = source.join(&name);
        let kind = fs::symlink_metadata(&object)?.file_type();
        ensure!(
            !kind.is_symlink(),
            "engine contract tree refuses link {}",
            object.display()
        );
        ensure!(
            !unclean(&text),
            "engine contract source was not stopped: {}",
            object.display()
        );
        if runtime_lock(&text) {
            ensure!(!kind.is_dir(), "engine contract tree has a lock directory");
            continue;
        }
        ensure!(
            entries.len() < MAX_ENTRIES,
            "engine contract tree has too many entries"
        );
        path.push(text);
        if kind.is_dir() {
            let child = destination
                .map(|parent| parent.create_private_directory(&name))
                .transpose()?;
            entries.push(Entry::Directory { path: path.clone() });
            walk_into(&object, child.as_ref(), path, entries, visit)?;
        } else if kind.is_file() {
            // Checked opens refuse links, including extra hard links.
            let mut input = directory.read(&name)?;
            let mut bytes = Vec::new();
            (&mut input).take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= FILE_LIMIT,
                "engine contract file exceeds size limit"
            );
            directory.verify(&name, &input)?;
            if let Some(parent) = destination {
                let mut output = parent.create_new(&name)?;
                output.write_all(&bytes)?;
                output.sync_all()?;
            }
            let joined = path.join("/");
            visit(&joined, &bytes)?;
            entries.push(Entry::File {
                path: path.clone(),
                bytes: bytes.len() as u64,
                sha256: hex(&Sha256::digest(&bytes)),
            });
        } else {
            bail!(
                "engine contract tree refuses non-regular object {}",
                object.display()
            );
        }
        path.pop();
    }
    directory.revalidate()?;
    Ok(())
}

/// Copy a stopped store's `data/` tree into the fresh private `destination`.
pub(crate) fn copy_data_tree(source: &Path, destination: &Directory) -> Result<Vec<Entry>> {
    walk(source, Some(destination), &mut |_, _| Ok(()))
}

/// A labelled byte pattern for [`scan`].
#[derive(Clone, Debug)]
pub(crate) struct Needle {
    pub(crate) label: String,
    pub(crate) bytes: Vec<u8>,
}

/// The UTF-8 and UTF-16LE encodings of `text`, plus its forward-slash and
/// backslash separator variants and, for a Windows verbatim path, the form
/// without its `\\?\` prefix, so a path is found in any spelling.
pub(crate) fn needles(label: &str, text: &str) -> Vec<Needle> {
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let mut spellings = vec![text.to_owned()];
    for variant in [text.replace('\\', "/"), text.replace('/', "\\")] {
        if !spellings.contains(&variant) {
            spellings.push(variant);
        }
    }
    let mut found = Vec::new();
    for spelling in spellings {
        let utf16: Vec<u8> = spelling.encode_utf16().flat_map(u16::to_le_bytes).collect();
        found.push(Needle {
            label: format!("{label} (UTF-8 {spelling:?})"),
            bytes: spelling.clone().into_bytes(),
        });
        found.push(Needle {
            label: format!("{label} (UTF-16LE {spelling:?})"),
            bytes: utf16,
        });
    }
    found
}

/// One needle found in one file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Hit {
    pub(crate) file: String,
    pub(crate) label: String,
}

/// The inventory of `root` and every needle occurrence in its files.
pub(crate) fn scan(root: &Path, needles: &[Needle]) -> Result<(Vec<Entry>, Vec<Hit>)> {
    for needle in needles {
        ensure!(
            !needle.bytes.is_empty(),
            "empty scan needle {}",
            needle.label
        );
    }
    let mut hits = Vec::new();
    let entries = walk(root, None, &mut |file, bytes| {
        for needle in needles {
            if bytes
                .windows(needle.bytes.len())
                .any(|window| window == needle.bytes.as_slice())
            {
                hits.push(Hit {
                    file: file.to_owned(),
                    label: needle.label.clone(),
                });
            }
        }
        Ok(())
    })?;
    Ok((entries, hits))
}

/// A stopped store's `data/` tree as produced on one host.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capture {
    pub(crate) format: u32,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) engine: String,
    pub(crate) project_scope: String,
    pub(crate) instance: String,
    pub(crate) main_head: String,
    pub(crate) usage_head: String,
    pub(crate) main_version: i32,
    pub(crate) usage_version: i32,
    pub(crate) entries: Vec<Entry>,
}

/// Write `data` (a stopped store's `data/`) and its record as the new private
/// capture directory `destination`. `capture.entries` is filled from the copy.
pub(crate) fn write_capture(
    data: &Path,
    destination: &Path,
    mut capture: Capture,
) -> Result<Capture> {
    ensure!(
        capture.format == CAPTURE_FORMAT && capture.entries.is_empty(),
        "invalid engine contract capture record"
    );
    let parent = files::parent(destination, Privacy::OwnerOnly, NameRetention::Movable)?;
    let root = parent.create_private_directory(files::name(destination)?)?;
    let tree = root.create_private_directory(OsStr::new(CAPTURE_DATA))?;
    capture.entries = copy_data_tree(data, &tree)?;
    ensure!(
        !capture.entries.is_empty(),
        "engine contract capture is empty"
    );
    let mut record = root.create_new(OsStr::new(CAPTURE_RECORD))?;
    record.write_all(&serde_json::to_vec_pretty(&capture)?)?;
    record.sync_all()?;
    Ok(capture)
}

/// Read only a capture's record, with inherited access (a CI download).
pub(crate) fn read_capture_record(capture: &Path) -> Result<Capture> {
    let root = files::open_directory(capture, Privacy::Inherited, NameRetention::Movable)?;
    let mut file = root.read(OsStr::new(CAPTURE_RECORD))?;
    let mut bytes = Vec::new();
    (&mut file).take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= RECORD_LIMIT,
        "engine contract capture record exceeds size limit"
    );
    root.verify(OsStr::new(CAPTURE_RECORD), &file)?;
    let record: Capture =
        serde_json::from_slice(&bytes).context("invalid engine contract capture record")?;
    ensure!(
        record.format == CAPTURE_FORMAT,
        "unsupported engine contract capture format {}",
        record.format
    );
    Ok(record)
}

/// Copy a capture's `data/` tree into the private store directory `store`
/// (which must not yet hold `data/`) and verify it against `record`, the
/// record [`read_capture_record`] returned for the same capture.
pub(crate) fn read_capture_tree(capture: &Path, record: &Capture, store: &Directory) -> Result<()> {
    let tree = store.create_private_directory(OsStr::new(CAPTURE_DATA))?;
    let entries = copy_data_tree(&capture.join(CAPTURE_DATA), &tree)?;
    ensure!(
        entries == record.entries,
        "engine contract capture tree does not match its record"
    );
    Ok(())
}

/// Why the cross-OS consumer did not run. Each cause prints a fixed token,
/// so a CI log (which captures the test's standard error) tells a missing
/// capture from a same-OS one without parsing prose.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NotRun {
    /// No capture directory was provided.
    NoCapture,
    /// The provided capture was produced on this OS (named).
    SameOs(String),
}

impl NotRun {
    pub(crate) fn token(&self) -> &'static str {
        match self {
            Self::NoCapture => "no-capture",
            Self::SameOs(_) => "same-os-capture",
        }
    }
}

impl std::fmt::Display for NotRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCapture => write!(
                formatter,
                "[{}] {CROSS_OS_CAPTURE} names no capture directory produced on another OS",
                self.token()
            ),
            Self::SameOs(os) => write!(
                formatter,
                "[{}] the provided capture was produced on this OS ({os}); a cross-OS run needs another",
                self.token()
            ),
        }
    }
}

/// Whether the cross-OS consumer runs, and why not when it does not.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CrossOs {
    NotRun(NotRun),
    Run {
        capture: PathBuf,
        record: Box<Capture>,
    },
}

/// Decide the cross-OS consumer from the provided capture directory. A
/// missing capture, or one produced on this OS, is an explicit not-run
/// result; `require` turns that into an error.
pub(crate) fn cross_os_plan(provided: Option<OsString>, require: bool) -> Result<CrossOs> {
    let plan = match provided {
        None => CrossOs::NotRun(NotRun::NoCapture),
        Some(path) => {
            let capture = PathBuf::from(path);
            let record = read_capture_record(&capture)
                .with_context(|| format!("read cross-OS capture {}", capture.display()))?;
            if record.os == std::env::consts::OS {
                CrossOs::NotRun(NotRun::SameOs(record.os))
            } else {
                CrossOs::Run {
                    capture,
                    record: Box::new(record),
                }
            }
        }
    };
    if require && let CrossOs::NotRun(reason) = &plan {
        bail!("{REQUIRE_CROSS_OS}=1 but the cross-OS engine contract did not run: {reason}");
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) -> Result<PathBuf> {
        let data = root.join("store/data");
        files::private_dir(&root.join("store"))?;
        files::private_dir(&data)?;
        let directory = files::directory(&data)?;
        let nested = directory.create_private_directory(OsStr::new(".dolt"))?;
        nested
            .create_new(OsStr::new("manifest"))?
            .write_all(b"engine manifest")?;
        nested.create_new(OsStr::new("LOCK"))?.write_all(b"")?;
        directory
            .create_new(OsStr::new("journal"))?
            .write_all(b"prefix KURU-CONTRACT-MARKER suffix")?;
        Ok(data)
    }

    fn record() -> Capture {
        Capture {
            format: CAPTURE_FORMAT,
            os: "not-this-os".into(),
            arch: std::env::consts::ARCH.into(),
            engine: crate::provision::DOLT_VERSION.into(),
            project_scope: format!("project/{}", "a".repeat(64)),
            instance: uuid::Uuid::nil().to_string(),
            main_head: "0".repeat(32),
            usage_head: "1".repeat(32),
            main_version: 7,
            usage_version: 4,
            entries: Vec::new(),
        }
    }

    /// The scanner finds an injected marker in either encoding and skips
    /// engine lock files, so a clean result elsewhere is a real negative.
    #[test]
    fn scan_detects_an_injected_marker_and_skips_lock_files() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let data = tree(root.path())?;
        let (entries, hits) = scan(&data, &needles("marker", "KURU-CONTRACT-MARKER"))?;
        assert_eq!(
            entries.iter().map(Entry::path).collect::<Vec<_>>(),
            [".dolt", ".dolt/manifest", "journal"]
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].file, "journal");
        assert!(hits[0].label.contains("UTF-8"), "{hits:?}");
        let utf16: Vec<u8> = "wide".encode_utf16().flat_map(u16::to_le_bytes).collect();
        files::directory(&data)?
            .create_new(OsStr::new("wide"))?
            .write_all(&utf16)?;
        let (_, hits) = scan(&data, &needles("wide", "wide"))?;
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].label.contains("UTF-16LE"), "{hits:?}");
        let separators = needles("path", "C:\\a/b");
        assert_eq!(separators.len(), 6, "{separators:?}");
        // A verbatim path is sought without its prefix, which also matches it.
        assert_eq!(needles("path", r"\\?\C:\a")[0].bytes, br"C:\a");
        assert!(
            scan(
                &data,
                &[Needle {
                    label: "empty".into(),
                    bytes: Vec::new()
                }]
            )
            .is_err()
        );
        root.release(Ok(()))
    }

    /// A stopped engine's server info refuses the copy.
    #[test]
    fn copy_refuses_a_running_engine_marker() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let data = tree(root.path())?;
        files::directory(&data)?
            .create_new(OsStr::new("sql-server.info"))?
            .write_all(b"1:3306:uuid")?;
        let destination = files::ensure_private_directory(&root.path().join("copy"))?;
        let error = copy_data_tree(&data, &destination).unwrap_err();
        assert!(
            format!("{error:#}").contains("was not stopped"),
            "{error:#}"
        );
        root.release(Ok(()))
    }

    /// The capture format round-trips, verifies every byte, and decides the
    /// cross-OS consumer explicitly.
    #[test]
    fn capture_round_trips_rejects_tampering_and_plans_the_cross_os_run() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let data = tree(root.path())?;
        let capture = root.path().join("capture");
        let written = write_capture(&data, &capture, record())?;
        assert_eq!(written.entries.len(), 3);
        let text = String::from_utf8(fs::read(capture.join(CAPTURE_RECORD))?)?;
        assert!(
            !text.contains(&root.path().to_string_lossy().to_string()),
            "capture record holds an absolute path: {text}"
        );
        let store = files::ensure_private_directory(&root.path().join("consumer"))?;
        let read = read_capture_record(&capture)?;
        assert_eq!(read, written);
        read_capture_tree(&capture, &read, &store)?;
        assert_eq!(
            fs::read(root.path().join("consumer/data/journal"))?,
            b"prefix KURU-CONTRACT-MARKER suffix"
        );

        // A changed byte, even of the same size, fails the record.
        let journal = files::directory(&capture.join(CAPTURE_DATA))?;
        let mut file = journal.read_write(OsStr::new("journal"))?;
        file.write_all(b"PREFIX")?;
        drop(file);
        let store = files::ensure_private_directory(&root.path().join("tampered"))?;
        let error =
            read_capture_tree(&capture, &read_capture_record(&capture)?, &store).unwrap_err();
        assert!(
            format!("{error:#}").contains("does not match its record"),
            "{error:#}"
        );

        match cross_os_plan(None, false)? {
            CrossOs::NotRun(reason) => {
                assert_eq!(reason, NotRun::NoCapture);
                assert_eq!(reason.token(), "no-capture");
                assert!(reason.to_string().starts_with("[no-capture] "), "{reason}");
                assert!(reason.to_string().contains(CROSS_OS_CAPTURE), "{reason}");
            }
            other => panic!("no capture must not run: {other:?}"),
        }
        let error = cross_os_plan(None, true).unwrap_err();
        assert!(format!("{error:#}").contains(REQUIRE_CROSS_OS), "{error:#}");
        assert!(format!("{error:#}").contains("[no-capture]"), "{error:#}");
        match cross_os_plan(Some(capture.clone().into_os_string()), true)? {
            CrossOs::Run {
                capture: path,
                record,
            } => {
                assert_eq!(path, capture);
                assert_eq!(record.os, "not-this-os");
            }
            other => panic!("a capture from another OS must run: {other:?}"),
        }
        let same = root.path().join("same-os");
        let mut local = record();
        local.os = std::env::consts::OS.into();
        write_capture(&data, &same, local)?;
        match cross_os_plan(Some(same.clone().into_os_string()), false)? {
            CrossOs::NotRun(reason) => {
                assert_eq!(reason, NotRun::SameOs(std::env::consts::OS.into()));
                assert_eq!(reason.token(), "same-os-capture");
                assert!(
                    reason.to_string().starts_with("[same-os-capture] "),
                    "{reason}"
                );
            }
            other => panic!("a same-OS capture must not run: {other:?}"),
        }
        let error = cross_os_plan(Some(same.into_os_string()), true).unwrap_err();
        assert!(
            format!("{error:#}").contains("[same-os-capture]"),
            "{error:#}"
        );
        let mut bad = record();
        bad.format = CAPTURE_FORMAT + 1;
        files::write(&capture.join(CAPTURE_RECORD), &serde_json::to_vec(&bad)?)?;
        assert!(read_capture_record(&capture).is_err());
        root.release(Ok(()))
    }
}
