//! Exact, bounded shell and manual support paired with one native executable.

use crate::{
    archive,
    targets::{ArchiveFormat, Target},
};
use anyhow::{Context, Result, ensure};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use kuru_archive::zip::{self, Archive, Limits, MemberKind, MemberSpec, WriteMember};
use kuru_platform::fs::{
    Directory, NameRetention, Privacy, Publication, PublicationPhase, regular_file_info,
};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

pub const NAMES: [&str; 5] = [
    "completions/kuru.bash",
    "completions/_kuru",
    "completions/kuru.fish",
    "completions/kuru.ps1",
    "man/kuru.1",
];
pub const MAX_FILE_BYTES: usize = 512 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Files(pub [Vec<u8>; NAMES.len()]);

impl Files {
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        NAMES
            .iter()
            .position(|item| *item == name)
            .map(|index| self.0[index].as_slice())
    }
}

pub fn archive_name(version: &str, target: &str) -> Result<String> {
    let version = archive::checked_version(version)?;
    let target = crate::targets::find(target)?;
    Ok(format!(
        "kuru-{version}-{}-shell-support.{}",
        target.triple,
        target.format.extension()
    ))
}

pub fn read_generated(root: &Path) -> Result<Files> {
    let root = Directory::open(root, Privacy::Inherited, NameRetention::Movable)?;
    exact_names(&root, &["completions", "man"], false)?;
    let completions = Directory::open(
        &root.path().join("completions"),
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let man = Directory::open(
        &root.path().join("man"),
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    exact_names(
        &completions,
        &["kuru.bash", "_kuru", "kuru.fish", "kuru.ps1"],
        true,
    )?;
    exact_names(&man, &["kuru.1"], true)?;
    let mut files: [Vec<u8>; NAMES.len()] = std::array::from_fn(|_| Vec::new());
    for (index, path) in NAMES.iter().enumerate() {
        let (directory, name) = if let Some(name) = path.strip_prefix("completions/") {
            (&completions, name)
        } else {
            (&man, path.strip_prefix("man/").expect("fixed name"))
        };
        let name = OsStr::new(name);
        let mut input = directory.read(name)?;
        let before = regular_file_info(&input)?;
        ensure!(
            before.len > 0 && before.len <= MAX_FILE_BYTES as u64,
            "shell support file has invalid size"
        );
        Read::by_ref(&mut input)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut files[index])?;
        ensure!(
            files[index].len() as u64 == before.len,
            "shell support file changed while reading"
        );
        input.rewind()?;
        let mut second = Vec::new();
        Read::by_ref(&mut input)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut second)?;
        ensure!(
            second == files[index],
            "shell support file changed while reading"
        );
        directory.verify(name, &input)?;
        ensure!(
            regular_file_info(&input)?.identity == before.identity,
            "shell support file changed while reading"
        );
    }
    Ok(Files(files))
}

fn exact_names(directory: &Directory, expected: &[&str], files: bool) -> Result<()> {
    let mut actual = HashSet::new();
    for entry in fs::read_dir(directory.path())? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("shell support has a non-UTF-8 name")?;
        let kind = entry.file_type()?;
        ensure!(
            expected.contains(&name)
                && if files { kind.is_file() } else { kind.is_dir() }
                && actual.insert(name.to_owned()),
            "shell support input inventory is unsafe or incomplete"
        );
    }
    directory.revalidate()?;
    ensure!(
        actual.len() == expected.len(),
        "shell support input inventory is incomplete"
    );
    Ok(())
}

fn limits() -> Limits {
    Limits {
        max_compressed_bytes: MAX_ENVELOPE_BYTES as u64,
        max_expanded_bytes: MAX_ENVELOPE_BYTES as u64,
        allow_ntfs_timestamps: false,
    }
}

pub fn encode(files: &Files, target: &Target) -> Result<Vec<u8>> {
    ensure!(
        files
            .0
            .iter()
            .all(|bytes| !bytes.is_empty() && bytes.len() <= MAX_FILE_BYTES),
        "shell support file has invalid size"
    );
    let bytes = match target.format {
        ArchiveFormat::Zip => {
            let members = NAMES
                .iter()
                .zip(&files.0)
                .map(|(name, bytes)| WriteMember {
                    name,
                    kind: MemberKind::File,
                    bytes,
                    executable: false,
                })
                .collect::<Vec<_>>();
            zip::write(&members, limits())?
        }
        ArchiveFormat::TarGz => {
            let mut output = Vec::new();
            {
                let gzip = GzBuilder::new()
                    .mtime(0)
                    .write(&mut output, Compression::default());
                let mut stream = tar::Builder::new(gzip);
                for (name, content) in NAMES.iter().zip(&files.0) {
                    let mut header = tar::Header::new_ustar();
                    header.set_size(content.len() as u64);
                    header.set_mode(0o644);
                    header.set_uid(0);
                    header.set_gid(0);
                    header.set_mtime(0);
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_cksum();
                    stream.append_data(&mut header, *name, content.as_slice())?;
                }
                stream.into_inner()?.finish()?;
            }
            output
        }
    };
    ensure!(
        bytes.len() <= MAX_ENVELOPE_BYTES,
        "shell support envelope exceeds size limit"
    );
    ensure!(
        decode(&bytes, target)? == *files,
        "shell support archive did not round-trip"
    );
    Ok(bytes)
}

pub fn decode(bytes: &[u8], target: &Target) -> Result<Files> {
    ensure!(
        bytes.len() <= MAX_ENVELOPE_BYTES,
        "shell support envelope exceeds size limit"
    );
    let mut files: [Option<Vec<u8>>; NAMES.len()] = std::array::from_fn(|_| None);
    if target.format == ArchiveFormat::Zip {
        let expected = NAMES.map(|name| MemberSpec {
            name,
            kind: MemberKind::File,
            max_bytes: MAX_FILE_BYTES as u64,
            exact_bytes: None,
            unix_mode: Some(0o100644),
        });
        let mut archive = Archive::open(bytes, &expected, limits())?;
        for (index, name) in NAMES.iter().enumerate() {
            let mut content = Vec::new();
            archive.copy(name, &mut content)?;
            files[index] = Some(content);
        }
    } else {
        let mut expanded = Vec::new();
        Read::by_ref(&mut GzDecoder::new(bytes))
            .take(MAX_ENVELOPE_BYTES as u64 + 1)
            .read_to_end(&mut expanded)?;
        ensure!(
            expanded.len() <= MAX_ENVELOPE_BYTES,
            "expanded shell support envelope exceeds size limit"
        );
        let mut tar = tar::Archive::new(expanded.as_slice());
        let mut seen = HashSet::new();
        for entry in tar.entries()?.raw(true) {
            let mut entry = entry?;
            let name = entry.path_bytes().to_vec();
            let index = NAMES
                .iter()
                .position(|expected| expected.as_bytes() == name)
                .context("shell support envelope contains an unexpected member")?;
            ensure!(
                entry.header().entry_type().is_file()
                    && entry.header().mode()? == 0o644
                    && entry.size() > 0
                    && entry.size() <= MAX_FILE_BYTES as u64
                    && seen.insert(index),
                "shell support envelope has unsafe or duplicate members"
            );
            let mut content = Vec::new();
            Read::by_ref(&mut entry)
                .take(MAX_FILE_BYTES as u64 + 1)
                .read_to_end(&mut content)?;
            ensure!(
                content.len() as u64 == entry.size(),
                "shell support member is truncated"
            );
            files[index] = Some(content);
        }
    }
    let complete = files
        .into_iter()
        .map(|file| {
            file.filter(|bytes| !bytes.is_empty())
                .context("shell support envelope is incomplete")
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Files(complete.try_into().expect("fixed inventory length")))
}

pub async fn verified(base: &str, version: &str, target: &str, manifest: &[u8]) -> Result<Files> {
    let name = archive_name(version, target)?;
    let expected = archive::expected_digest(manifest, &name)?;
    let bytes = archive::read_asset(base, &name, MAX_ENVELOPE_BYTES).await?;
    ensure!(
        archive::digest(&bytes) == expected,
        "shell support checksum mismatch; executable unchanged"
    );
    decode(&bytes, crate::targets::find(target)?)
}

pub fn package(input: &Path, target: &str, version: &str, output: &Path) -> Result<PathBuf> {
    let target = crate::targets::find(target)?;
    let files = read_generated(input)?;
    let bytes = encode(&files, target)?;
    let name = archive_name(version, target.triple)?;
    let output = archive::destination_directory(output)?;
    let stage = crate::staging::Stage::create(&output, ".kuru-support-")?;
    let mut candidate = stage.directory().create_new(OsStr::new(&name))?;
    candidate.write_all(&bytes)?;
    candidate.sync_all()?;
    let checksum_name = format!("{name}.sha256");
    let mut checksum = stage.directory().create_new(OsStr::new(&checksum_name))?;
    writeln!(checksum, "{}  {name}", archive::digest(&bytes))?;
    checksum.sync_all()?;
    output.publish_file(
        stage.directory(),
        OsStr::new(&name),
        &candidate,
        OsStr::new(&name),
        Publication::ReplaceRegular,
    )?;
    output.publish_file(
        stage.directory(),
        OsStr::new(&checksum_name),
        &checksum,
        OsStr::new(&checksum_name),
        Publication::ReplaceRegular,
    )?;
    drop(candidate);
    drop(checksum);
    stage.finish()?;
    let path = output.path().join(name);
    ensure!(
        fs::metadata(&path)?.len() <= MAX_ENVELOPE_BYTES as u64,
        "published shell support envelope exceeds size limit"
    );
    Ok(path)
}

fn private_child(parent: &Directory, name: &str) -> Result<Directory> {
    match parent.create_private_directory(OsStr::new(name)) {
        Ok(directory) => Ok(directory),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(Directory::open(
            &parent.path().join(name),
            Privacy::OwnerOnly,
            NameRetention::Movable,
        )?),
        Err(error) => Err(error.into()),
    }
}

/// Publish immutable versioned files before the executable changes. An existing
/// target snapshot is reused only when its entire inventory and bytes match.
pub fn install_versioned(
    files: &Files,
    root: &Path,
    version: &str,
    target: &str,
) -> Result<PathBuf> {
    let version = archive::checked_version(version)?;
    let target = crate::targets::find(target)?;
    let root = archive::destination_directory(root)?;
    let share = private_child(&root, "share")?;
    let kuru = private_child(&share, "kuru")?;
    let version_dir = private_child(&kuru, version)?;
    let path = version_dir.path().join(target.triple);
    match version_dir.create_private_directory(OsStr::new(target.triple)) {
        Ok(new) => {
            let result = (|| -> Result<()> {
                let completions = new.create_private_directory(OsStr::new("completions"))?;
                let man = new.create_private_directory(OsStr::new("man"))?;
                for (name, bytes) in NAMES.iter().zip(&files.0) {
                    let (parent, leaf) = if let Some(leaf) = name.strip_prefix("completions/") {
                        (&completions, leaf)
                    } else {
                        (&man, name.strip_prefix("man/").expect("fixed name"))
                    };
                    let leaf = OsStr::new(leaf);
                    let mut output = parent.create_new(leaf)?;
                    output.write_all(bytes)?;
                    output.sync_all()?;
                    parent.verify(leaf, &output)?;
                }
                drop(completions);
                drop(man);
                ensure!(
                    read_generated(new.path())? == *files,
                    "installed shell support differs from verified release"
                );
                Ok(())
            })();
            if let Err(error) = result {
                new.remove_tree()
                    .context("remove incomplete owned shell support snapshot")?;
                return Err(error);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = Directory::open(&path, Privacy::OwnerOnly, NameRetention::Movable)?;
            ensure!(
                read_generated(existing.path())? == *files,
                "existing shell support snapshot differs from verified release"
            );
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

/// This stable ordinary man path is advanced only after confirmed executable
/// publication. A failed publication is reported as a partial support result.
pub fn publish_stable_man(files: &Files, root: &Path) -> Result<PathBuf> {
    let root = archive::destination_directory(root)?;
    let share = private_child(&root, "share")?;
    let man = private_child(&share, "man")?;
    let man1 = private_child(&man, "man1")?;
    let stage = crate::staging::Stage::create(&man1, ".kuru-man-")?;
    let name = OsStr::new("kuru.1");
    let mut output = stage.directory().create_new(name)?;
    output.write_all(files.get("man/kuru.1").expect("fixed inventory"))?;
    output.sync_all()?;
    if let Err(error) = man1.publish_file(
        stage.directory(),
        name,
        &output,
        name,
        Publication::ReplaceRegular,
    ) && (error.phase != PublicationPhase::Uncertain || man1.verify(name, &output).is_err())
    {
        return Err(error.into());
    }
    drop(output);
    stage.finish()?;
    Ok(man1.path().join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Files {
        Files(std::array::from_fn(|index| {
            format!("generated {index}\n").into_bytes()
        }))
    }

    #[test]
    fn target_paired_envelopes_are_reproducible_and_exact() {
        for target in &crate::targets::CATALOG {
            let input = files();
            let encoded = encode(&input, target).unwrap();
            assert_eq!(encoded, encode(&input, target).unwrap());
            assert_eq!(decode(&encoded, target).unwrap(), input);
            let name = archive_name("v0.8.0", target.triple).unwrap();
            assert!(name.contains(&format!("{}-shell-support.", target.triple)));
            assert!(!name.ends_with(&format!("-{}.{}", target.triple, target.format.extension())));
        }
    }

    #[test]
    fn support_decoder_rejects_extra_duplicate_and_oversized_members() {
        let input = files();
        let unix = crate::targets::find(crate::targets::TARGETS[0]).unwrap();
        let windows = crate::targets::find(crate::targets::TARGETS[4]).unwrap();
        let malformed = zip::write(
            &[
                WriteMember {
                    name: NAMES[0],
                    kind: MemberKind::File,
                    bytes: b"one",
                    executable: false,
                },
                WriteMember {
                    name: "other/file",
                    kind: MemberKind::File,
                    bytes: b"unexpected",
                    executable: false,
                },
            ],
            limits(),
        )
        .unwrap();
        assert!(decode(&malformed, windows).is_err());

        let mut bytes = Vec::new();
        {
            let gzip = GzBuilder::new()
                .mtime(0)
                .write(&mut bytes, Compression::default());
            let mut stream = tar::Builder::new(gzip);
            for (name, content) in NAMES
                .iter()
                .zip(&input.0)
                .chain(std::iter::once((&NAMES[0], &input.0[0])))
            {
                let mut header = tar::Header::new_ustar();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                stream
                    .append_data(&mut header, *name, content.as_slice())
                    .unwrap();
            }
            stream.into_inner().unwrap().finish().unwrap();
        }
        assert!(decode(&bytes, unix).is_err());

        let mut oversized = input;
        oversized.0[0] = vec![b'x'; MAX_FILE_BYTES + 1];
        assert!(encode(&oversized, unix).is_err());
        assert!(encode(&oversized, windows).is_err());
    }

    #[test]
    fn generated_inventory_and_versioned_publication_reject_substitution() {
        let root = tempfile::tempdir().unwrap();
        let generated = root.path().join("generated");
        fs::create_dir_all(generated.join("completions")).unwrap();
        fs::create_dir_all(generated.join("man")).unwrap();
        let input = files();
        for (name, bytes) in NAMES.iter().zip(&input.0) {
            fs::write(generated.join(name), bytes).unwrap();
        }
        assert_eq!(read_generated(&generated).unwrap(), input);
        fs::write(generated.join("completions/extra"), b"extra").unwrap();
        assert!(read_generated(&generated).is_err());
        fs::remove_file(generated.join("completions/extra")).unwrap();

        let install = root.path().join("bin");
        fs::create_dir(&install).unwrap();
        let target = crate::targets::TARGETS[0];
        let snapshot = install_versioned(&input, &install, "0.8.0", target).unwrap();
        assert_eq!(read_generated(&snapshot).unwrap(), input);
        assert_eq!(
            install_versioned(&input, &install, "0.8.0", target).unwrap(),
            snapshot
        );
        let mut altered = input.clone();
        altered.0[0] = b"different\n".to_vec();
        assert!(install_versioned(&altered, &install, "0.8.0", target).is_err());
        let stable = publish_stable_man(&input, &install).unwrap();
        assert_eq!(fs::read(stable).unwrap(), input.get("man/kuru.1").unwrap());
    }
}
