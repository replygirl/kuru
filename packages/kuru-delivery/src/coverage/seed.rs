//! The allow-listed dependency seed of a fresh partition target.
//!
//! A partition still compiles into a fresh target. Before its first build it
//! may import cached artifacts of non-workspace packages from a restored seed,
//! so Cargo can reuse dependency units it judges fresh and rebuilds anything
//! else. Only `debug/{deps,build,.fingerprint}` entries named
//! `<stem>-<16 hex>` whose stem is not a workspace package or crate are
//! accepted; links, raw profiles, private state and malformed entries are
//! refused and counted. An absent or evicted seed only makes the build slower.
//! Export applies the same allow-list, so a saved seed holds exactly what an
//! import accepts.

use super::ledger::Transfer;
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::Path,
};

/// The Cargo profile directory of the test build.
pub const PROFILE: &str = "debug";
const SEEDED: [&str; 3] = [".fingerprint", "build", "deps"];
const DEPS_EXTENSIONS: [&str; 10] = [
    "a", "d", "dll", "dylib", "exp", "lib", "pdb", "rlib", "rmeta", "so",
];
const PRIVATE: [&str; 2] = ["kuru-shard-state", "kuru-test-supervisors"];
const PARTIAL: &str = ".kuru-seed-partial";

/// Why a seed entry was not transferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    Symlink,
    Workspace,
    Profile,
    Private,
    Shape,
    Present,
}

impl Refusal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Symlink => "symlink",
            Self::Workspace => "workspace",
            Self::Profile => "profile",
            Self::Private => "private",
            Self::Shape => "shape",
            Self::Present => "present",
        }
    }
}

/// Entries and bytes transferred and entries refused by reason.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SeedReport {
    pub transferred: Transfer,
    pub refused: BTreeMap<String, u64>,
}

impl SeedReport {
    fn refuse(&mut self, reason: Refusal) {
        *self.refused.entry(reason.name().to_owned()).or_default() += 1;
    }
}

/// Every name a workspace package or crate may give its outputs: package and
/// target names in hyphen and underscore forms.
pub fn workspace_names<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    names
        .into_iter()
        .flat_map(|name| [name.to_owned(), name.replace('-', "_")])
        .collect()
}

/// Split `<stem>-<16 hex>[.ext...]`.
fn parse_entry(name: &str) -> Option<(&str, Option<&str>)> {
    let (stem, rest) = name.rsplit_once('-')?;
    let (hash, extension) = match rest.split_once('.') {
        Some((hash, extension)) => (hash, Some(extension)),
        None => (rest, None),
    };
    (!stem.is_empty()
        && hash.len() == 16
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some((stem, extension))
}

/// Classify one entry of `debug/<kind>` by its name alone.
fn classify(kind: &str, name: &str, workspace: &BTreeSet<String>) -> Result<(), Refusal> {
    let Some((stem, extension)) = parse_entry(name) else {
        return Err(if name.ends_with(".profraw") {
            Refusal::Profile
        } else if PRIVATE.contains(&name) {
            Refusal::Private
        } else {
            Refusal::Shape
        });
    };
    let underscored = stem.replace('-', "_");
    let unprefixed = underscored.strip_prefix("lib");
    if workspace.contains(stem)
        || workspace.contains(&underscored)
        || unprefixed.is_some_and(|stem| workspace.contains(stem))
    {
        return Err(Refusal::Workspace);
    }
    match (kind, extension) {
        ("deps", Some(extension))
            if extension
                .split('.')
                .all(|part| DEPS_EXTENSIONS.contains(&part)) =>
        {
            Ok(())
        }
        ("build" | ".fingerprint", None) => Ok(()),
        (_, Some(extension)) if extension.ends_with("profraw") => Err(Refusal::Profile),
        _ => Err(Refusal::Shape),
    }
}

/// Inspect an accepted entry without following links: every descendant must
/// be a regular file or directory, with no raw profile or private name.
/// Returns its total file bytes.
fn inspect(path: &Path, expect_directory: bool) -> Result<u64, Refusal> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Refusal::Shape)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || is_reparse_point(&metadata) {
        return Err(Refusal::Symlink);
    }
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if PRIVATE.contains(&name) {
        return Err(Refusal::Private);
    }
    if name.ends_with(".profraw") {
        return Err(Refusal::Profile);
    }
    if !expect_directory {
        return if file_type.is_file() {
            Ok(metadata.len())
        } else {
            Err(Refusal::Shape)
        };
    }
    if !file_type.is_dir() {
        return Err(Refusal::Shape);
    }
    let mut bytes = 0_u64;
    for entry in fs::read_dir(path).map_err(|_| Refusal::Shape)? {
        let entry = entry.map_err(|_| Refusal::Shape)?;
        let child = entry.path();
        let child_type = fs::symlink_metadata(&child)
            .map_err(|_| Refusal::Shape)?
            .file_type();
        bytes = bytes.saturating_add(inspect(&child, child_type.is_dir())?);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

/// Copy a tree preserving file modification times, for a move across devices.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination)?;
        fs::OpenOptions::new()
            .write(true)
            .open(destination)?
            .set_modified(metadata.modified()?)?;
    }
    Ok(())
}

/// Move one entry, copying through a private temporary name across devices.
fn move_entry(source: &Path, destination: &Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            let mut partial = destination.as_os_str().to_owned();
            partial.push(PARTIAL);
            let partial = std::path::PathBuf::from(partial);
            copy_tree(source, &partial)
                .and_then(|()| fs::rename(&partial, destination).map_err(Into::into))
                .with_context(|| format!("copy seed entry {}", source.display()))
        }
        Err(error) => {
            Err(anyhow::Error::new(error).context(format!("move seed entry {}", source.display())))
        }
    }
}

/// Transfer every allow-listed entry of `from/debug/<kind>` into
/// `to/debug/<kind>`, counting refusals. Entries already present in the
/// destination are refused, never replaced.
fn transfer(from: &Path, to: &Path, workspace: &BTreeSet<String>) -> Result<SeedReport> {
    let mut report = SeedReport::default();
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = fs::symlink_metadata(entry.path())?;
        if name != PROFILE {
            report.refuse(if metadata.file_type().is_symlink() {
                Refusal::Symlink
            } else {
                Refusal::Shape
            });
            continue;
        }
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            report.refuse(Refusal::Symlink);
        } else if !metadata.is_dir() {
            report.refuse(Refusal::Shape);
        }
    }
    let profile = from.join(PROFILE);
    if !fs::symlink_metadata(&profile)
        .is_ok_and(|metadata| metadata.is_dir() && !is_reparse_point(&metadata))
    {
        return Ok(report);
    }
    for entry in fs::read_dir(&profile)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = name.to_str().unwrap_or_default();
        let metadata = fs::symlink_metadata(entry.path())?;
        if !SEEDED.contains(&kind) {
            report.refuse(if PRIVATE.contains(&kind) {
                Refusal::Private
            } else if metadata.file_type().is_symlink() {
                Refusal::Symlink
            } else {
                Refusal::Shape
            });
            continue;
        }
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() {
            report.refuse(Refusal::Symlink);
            continue;
        }
        let destination = to.join(PROFILE).join(kind);
        fs::create_dir_all(&destination)
            .with_context(|| format!("create {}", destination.display()))?;
        let mut names: Vec<_> = fs::read_dir(entry.path())?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<_>>()?;
        names.sort();
        for name in names {
            let source = entry.path().join(&name);
            let Some(text) = name.to_str() else {
                report.refuse(Refusal::Shape);
                continue;
            };
            let accepted =
                classify(kind, text, workspace).and_then(|()| inspect(&source, kind != "deps"));
            let bytes = match accepted {
                Ok(bytes) => bytes,
                Err(reason) => {
                    report.refuse(reason);
                    continue;
                }
            };
            let target = destination.join(&name);
            if fs::symlink_metadata(&target).is_ok() {
                report.refuse(Refusal::Present);
                continue;
            }
            move_entry(&source, &target)?;
            report.transferred.entries += 1;
            report.transferred.bytes = report.transferred.bytes.saturating_add(bytes);
        }
    }
    Ok(report)
}

/// Import a restored seed into a fresh target. `None` is returned when the
/// seed directory does not exist, which is a miss and never an error.
pub fn import(
    seed: &Path,
    target: &Path,
    workspace: &BTreeSet<String>,
) -> Result<Option<SeedReport>> {
    match fs::symlink_metadata(seed) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::Error::new(error).context(format!("inspect {}", seed.display()))),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            let mut report = SeedReport::default();
            report.refuse(Refusal::Symlink);
            Ok(Some(report))
        }
        Ok(_) => transfer(seed, target, workspace).map(Some),
    }
}

/// Export the allow-listed dependency entries of a finished target into a new
/// seed directory, which must not exist.
pub fn export(
    target: &Path,
    destination: &Path,
    workspace: &BTreeSet<String>,
) -> Result<SeedReport> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::create_dir(destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("seed export already exists: {}", destination.display())
        } else {
            anyhow::Error::new(error).context(format!("create {}", destination.display()))
        }
    })?;
    transfer(target, destination, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const HASH: &str = "0123456789abcdef";

    fn workspace() -> BTreeSet<String> {
        workspace_names([
            "kuru-core",
            "kuru-memory",
            "kuru_memory",
            "release_workflow",
        ])
    }

    fn file(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// A seed with every accepted and refused shape.
    fn seed(root: &Path) {
        let debug = root.join(PROFILE);
        file(&debug.join(format!("deps/libserde-{HASH}.rlib")), b"rlib");
        file(&debug.join(format!("deps/serde-{HASH}.d")), b"d");
        file(
            &debug.join(format!("deps/serde_derive-{HASH}.dll.lib")),
            b"lib",
        );
        file(
            &debug.join(format!("build/proc-macro2-{HASH}/out/x.rs")),
            b"out",
        );
        file(
            &debug.join(format!("build/proc-macro2-{HASH}/output")),
            b"o",
        );
        file(
            &debug.join(format!(".fingerprint/serde-{HASH}/lib-serde")),
            b"f",
        );
        // Refused: workspace crates in every name form.
        file(&debug.join(format!("deps/libkuru_core-{HASH}.rlib")), b"w");
        file(&debug.join(format!("deps/kuru_memory-{HASH}")), b"w");
        file(&debug.join(format!("deps/release_workflow-{HASH}.d")), b"w");
        file(
            &debug.join(format!("build/kuru-memory-{HASH}/output")),
            b"w",
        );
        file(
            &debug.join(format!(".fingerprint/kuru-core-{HASH}/lib")),
            b"w",
        );
        // Refused: profiles, private state, malformed names and extensions.
        file(&debug.join("deps/kuru-1-2.profraw"), b"p");
        file(&debug.join(format!("build/cc-{HASH}/out/x.profraw")), b"p");
        file(
            &debug.join(format!("build/cc2-{HASH}/kuru-shard-state/x")),
            b"s",
        );
        file(&debug.join("kuru-test-supervisors/current.json"), b"s");
        file(&debug.join("deps/serde-XYZ.rlib"), b"m");
        file(&debug.join(format!("deps/serde-{HASH}.exe")), b"m");
        file(&debug.join(format!("deps/serde-{HASH}")), b"m");
        file(&debug.join(format!("build/cc3-{HASH}")), b"file-not-dir");
        file(&debug.join("incremental/x"), b"m");
        file(&root.join("release/deps/x"), b"m");
        file(&root.join(".rustc_info.json"), b"m");
    }

    fn names(root: &Path) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    found.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        found
    }

    fn accepted() -> BTreeSet<String> {
        [
            format!("debug/deps/libserde-{HASH}.rlib"),
            format!("debug/deps/serde-{HASH}.d"),
            format!("debug/deps/serde_derive-{HASH}.dll.lib"),
            format!("debug/build/proc-macro2-{HASH}/out/x.rs"),
            format!("debug/build/proc-macro2-{HASH}/output"),
            format!("debug/.fingerprint/serde-{HASH}/lib-serde"),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn import_takes_only_non_workspace_dependency_entries() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("seed");
        let target = temp.path().join("target");
        seed(&source);
        fs::create_dir(&target).unwrap();
        let report = import(&source, &target, &workspace()).unwrap().unwrap();
        assert_eq!(names(&target), accepted());
        // Five entries: three dependency files and two directories.
        assert_eq!(report.transferred.entries, 5);
        assert_eq!(report.transferred.bytes, 4 + 1 + 3 + 3 + 1 + 1);
        let refused: BTreeMap<_, _> = report
            .refused
            .iter()
            .map(|(name, count)| (name.as_str(), *count))
            .collect();
        assert_eq!(
            refused,
            BTreeMap::from([
                ("private", 2),
                ("profile", 2),
                ("shape", 7),
                ("workspace", 5),
            ])
        );
        // Accepted entries moved; refused ones stayed behind.
        assert!(
            !source
                .join(format!("debug/deps/libserde-{HASH}.rlib"))
                .exists()
        );
        assert!(
            source
                .join(format!("debug/deps/libkuru_core-{HASH}.rlib"))
                .exists()
        );
    }

    #[test]
    fn absent_seeds_are_misses_and_links_are_refused() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        assert_eq!(
            import(&temp.path().join("missing"), &target, &workspace()).unwrap(),
            None
        );
        // A seed without a profile directory imports nothing.
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        let report = import(&empty, &target, &workspace()).unwrap().unwrap();
        assert_eq!(report, SeedReport::default());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let source = temp.path().join("linked");
            let outside = temp.path().join("outside");
            file(&outside.join("secret"), b"x");
            file(&source.join(format!("debug/deps/libok-{HASH}.rlib")), b"ok");
            fs::create_dir_all(source.join("debug/build")).unwrap();
            symlink(&outside, source.join(format!("debug/build/cc-{HASH}"))).unwrap();
            fs::create_dir_all(source.join(format!("debug/build/cc2-{HASH}"))).unwrap();
            symlink(
                outside.join("secret"),
                source.join(format!("debug/build/cc2-{HASH}/output")),
            )
            .unwrap();
            symlink(
                outside.join("secret"),
                source.join(format!("debug/deps/libln-{HASH}.rlib")),
            )
            .unwrap();
            symlink(&outside, source.join(".fingerprint-link")).unwrap();
            let report = import(&source, &target, &workspace()).unwrap().unwrap();
            assert_eq!(report.transferred.entries, 1);
            assert_eq!(report.refused.get("symlink"), Some(&4));
            // The seed root itself may not be a link.
            let root_link = temp.path().join("root-link");
            symlink(&outside, &root_link).unwrap();
            let report = import(&root_link, &target, &workspace()).unwrap().unwrap();
            assert_eq!(report.refused.get("symlink"), Some(&1));
            assert_eq!(report.transferred, Transfer::default());
        }
    }

    #[test]
    fn existing_target_entries_are_never_replaced() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("seed");
        let target = temp.path().join("target");
        file(
            &source.join(format!("debug/deps/libserde-{HASH}.rlib")),
            b"seed",
        );
        file(
            &target.join(format!("debug/deps/libserde-{HASH}.rlib")),
            b"own",
        );
        let report = import(&source, &target, &workspace()).unwrap().unwrap();
        assert_eq!(report.refused.get("present"), Some(&1));
        assert_eq!(
            fs::read(target.join(format!("debug/deps/libserde-{HASH}.rlib"))).unwrap(),
            b"own"
        );
    }

    #[test]
    fn export_writes_exactly_what_import_accepts() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        seed(&target);
        let destination = temp.path().join("export");
        let report = export(&target, &destination, &workspace()).unwrap();
        assert_eq!(names(&destination), accepted());
        assert_eq!(report.transferred.entries, 5);
        // A second export refuses the existing destination.
        let error = export(&target, &destination, &workspace())
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
        // Re-importing the export accepts all of it.
        let fresh = temp.path().join("fresh");
        fs::create_dir(&fresh).unwrap();
        let again = import(&destination, &fresh, &workspace()).unwrap().unwrap();
        assert_eq!(again.transferred.entries, 5);
        assert!(again.refused.is_empty());
        assert_eq!(names(&fresh), accepted());
    }

    #[test]
    fn entry_names_parse_strictly() {
        assert_eq!(
            parse_entry(&format!("proc-macro2-{HASH}")),
            Some(("proc-macro2", None))
        );
        assert_eq!(
            parse_entry(&format!("libc-{HASH}.rlib")),
            Some(("libc", Some("rlib")))
        );
        for name in [
            "serde",
            &format!("-{HASH}"),
            "serde-0123456789ABCDEF",
            "serde-0123456789abcde",
            "serde-0123456789abcdef0",
        ] {
            assert_eq!(parse_entry(name), None, "{name}");
        }
        let workspace = workspace();
        // `libc` is a dependency, not the workspace crate `c`.
        assert_eq!(
            classify("deps", &format!("libc-{HASH}.rlib"), &workspace),
            Ok(())
        );
        assert_eq!(
            classify("deps", &format!("libkuru_memory-{HASH}.rmeta"), &workspace),
            Err(Refusal::Workspace)
        );
        assert_eq!(
            classify("build", &format!("serde-{HASH}.d"), &workspace),
            Err(Refusal::Shape)
        );
        assert_eq!(
            classify("deps", &format!("x-{HASH}.profraw"), &workspace),
            Err(Refusal::Profile)
        );
    }

    #[test]
    fn cross_device_moves_copy_through_a_private_name_and_keep_times() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("entry");
        file(&source.join("out/a"), b"a");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(source.join("out/a"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let copied = temp.path().join("copied");
        copy_tree(&source, &copied).unwrap();
        assert_eq!(fs::read(copied.join("out/a")).unwrap(), b"a");
        assert_eq!(
            fs::metadata(copied.join("out/a"))
                .unwrap()
                .modified()
                .unwrap(),
            old
        );
        // An ordinary move keeps the entry and leaves no partial name.
        let moved = temp.path().join("moved");
        move_entry(&copied, &moved).unwrap();
        assert!(moved.join("out/a").is_file());
        assert!(!copied.exists());
        assert!(move_entry(&temp.path().join("missing"), &temp.path().join("x")).is_err());
    }
}
