//! Reproducible release archives and bounded, atomic installation.

use crate::targets::{ArchiveFormat, Target};
use anyhow::{Context, Result, ensure};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use kuru_archive::zip::{self, Archive, Limits, MemberKind, MemberSpec, WriteMember};
use kuru_platform::fs::{
    Directory, FileInfo, NameRetention, Privacy, Publication, PublicationPhase, make_executable,
    regular_file_info, validate_component,
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub use crate::targets::TARGETS;
pub const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

pub fn checked_version(value: &str) -> Result<&str> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let (numbers, suffix) = value.split_once('-').unwrap_or((value, ""));
    let numbers: Vec<_> = numbers.split('.').collect();
    ensure!(
        numbers.len() == 3
            && numbers
                .iter()
                .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            && (!value.contains('-') || !suffix.is_empty())
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "version must be an explicit semantic version, for example 0.1.0"
    );
    Ok(value)
}

pub fn host_target() -> Result<&'static str> {
    Ok(crate::targets::host()?.triple)
}

pub fn archive_name(version: &str, target: &str) -> Result<String> {
    let version = checked_version(version)?;
    let target = crate::targets::find(target)?;
    Ok(format!(
        "kuru-{version}-{}.{}",
        target.triple,
        target.format.extension()
    ))
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn expected_digest(manifest: &[u8], filename: &str) -> Result<String> {
    let text = std::str::from_utf8(manifest).context("checksum manifest is not UTF-8")?;
    let matches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(' ')?;
            let name = name.strip_prefix(' ').or_else(|| name.strip_prefix('*'))?;
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && name == filename)
                .then(|| hash.to_ascii_lowercase())
        })
        .collect();
    ensure!(
        matches.len() == 1,
        "checksum manifest must name the release archive exactly once"
    );
    Ok(matches[0].clone())
}

pub async fn read_asset(base: &str, name: &str, limit: usize) -> Result<Vec<u8>> {
    kuru_platform::fs::validate_component(OsStr::new(name))?;
    let native_path = Path::new(base).is_absolute()
        || base.starts_with("\\\\")
        || (base.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && base.as_bytes().get(1) == Some(&b':'));
    if let Some(mut base) = url::Url::parse(base).ok().filter(|_| !native_path) {
        ensure!(
            base.scheme() == "https"
                && base.host_str().is_some()
                && base.username().is_empty()
                && base.password().is_none(),
            "release base must be HTTPS without embedded credentials"
        );
        ensure!(
            base.query().is_none() && base.fragment().is_none(),
            "release base must not include a query or fragment"
        );
        base.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid release base"))?
            .pop_if_empty()
            .push(name);
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(60))
            .build()?;
        let mut response = client.get(base).send().await?.error_for_status()?;
        ensure!(
            response.content_length().is_none_or(|n| n <= limit as u64),
            "release asset exceeds size limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "release asset exceeds size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    } else {
        let directory = Directory::open(
            &absolute(Path::new(base))?,
            Privacy::Inherited,
            NameRetention::Movable,
        )?;
        let mut input = directory.read(OsStr::new(name))?;
        let bytes = bounded(&mut input, limit, "release asset")?;
        directory.verify(OsStr::new(name), &input)?;
        Ok(bytes)
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn bounded(reader: impl Read, limit: usize, description: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "{description} exceeds size limit");
    Ok(bytes)
}

// Cargo can hard-link its public executable name to a compiled artifact. Read
// that source without changing it; installed files and publication destinations
// continue to use Directory's stricter single-link policy.
fn open_build_input(parent: &Directory, name: &OsStr) -> Result<File> {
    validate_component(name)?;
    let current = Directory::open(parent.path(), Privacy::Inherited, NameRetention::Movable)?;
    ensure!(
        current.identity() == parent.identity(),
        "binary parent changed"
    );
    let path = parent.path().join(name);
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file(),
        "binary must be a regular file, not a symlink or directory"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT also covers non-symlink reparse tags.
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "binary must not be a reparse point"
        );
    }
    let input = File::open(path)?;
    regular_file_info(&input)?;
    #[cfg(unix)]
    ensure!(
        input.metadata()?.permissions().mode() & 0o111 != 0,
        "binary must be an existing executable file"
    );
    Ok(input)
}

fn verify_build_snapshot(
    parent: &Directory,
    name: &OsStr,
    input: &mut File,
    before: FileInfo,
    bytes: &[u8],
) -> Result<()> {
    let current = open_build_input(parent, name)?;
    let after = regular_file_info(input)?;
    let named = regular_file_info(&current)?;
    ensure!(
        before.identity == after.identity
            && before.identity == named.identity
            && before.len == after.len
            && before.len == named.len
            && before.len == bytes.len() as u64,
        "binary changed while reading build input"
    );
    // Identity and length alone do not detect an in-place, same-size rebuild.
    // Compare a second bounded read with the exact bytes about to be packaged.
    input.rewind()?;
    let mut buffer = [0; 8192];
    for expected in bytes.chunks(buffer.len()) {
        input.read_exact(&mut buffer[..expected.len()])?;
        ensure!(
            &buffer[..expected.len()] == expected,
            "binary changed while reading build input"
        );
    }
    ensure!(
        input.read(&mut buffer[..1])? == 0,
        "binary changed while reading build input"
    );
    let final_named = open_build_input(parent, name)?;
    ensure!(
        regular_file_info(&final_named)?.identity == before.identity,
        "binary name changed while reading build input"
    );
    Ok(())
}

pub async fn install(
    base: &str,
    version: &str,
    destination: &Path,
    target: Option<&str>,
) -> Result<PathBuf> {
    let target = match target {
        Some(target) => target,
        None => host_target()?,
    };
    let bytes = verified_binary(base, version, target).await?;
    install_binary(&bytes, destination, crate::targets::find(target)?)
}

/// Install a trusted local build using the same bounded held-file publication
/// path as release archives. This never runs the source executable.
pub fn install_local(binary: &Path, destination: &Path, target: Option<&str>) -> Result<PathBuf> {
    let target = crate::targets::find(match target {
        Some(target) => target,
        None => host_target()?,
    })?;
    let binary = absolute(binary)?;
    let parent = Directory::open(
        binary.parent().context("binary has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let name = binary.file_name().context("binary has no filename")?;
    let mut source = open_build_input(&parent, name)?;
    let before = regular_file_info(&source)?;
    let bytes = bounded(&mut source, MAX_ARCHIVE_BYTES, "local executable")?;
    ensure!(!bytes.is_empty(), "local executable is empty");
    verify_build_snapshot(&parent, name, &mut source, before, &bytes)?;
    install_binary(&bytes, destination, target)
}

/// Verify release checksum and the complete archive before exposing candidate bytes.
/// This function never executes, installs or probes the candidate.
pub async fn verified_binary(base: &str, version: &str, target: &str) -> Result<Vec<u8>> {
    let name = archive_name(version, target)?;
    let expected = expected_digest(&read_asset(base, "SHA256SUMS", 64 * 1024).await?, &name)?;
    let bytes = read_asset(base, &name, MAX_ARCHIVE_BYTES).await?;
    ensure!(
        digest(&bytes) == expected,
        "release archive checksum mismatch; existing executable unchanged"
    );
    extract_binary(&bytes, crate::targets::find(target)?, MAX_ARCHIVE_BYTES)
}

#[cfg(test)]
fn install_archive(bytes: &[u8], destination: &Path, limit: usize) -> Result<PathBuf> {
    let target = crate::targets::find(TARGETS[0])?;
    install_binary(&extract_binary(bytes, target, limit)?, destination, target)
}

fn zip_limits(limit: usize) -> Limits {
    Limits {
        max_compressed_bytes: limit as u64,
        max_expanded_bytes: limit as u64,
        allow_ntfs_timestamps: false,
    }
}

fn extract_binary(bytes: &[u8], target: &Target, limit: usize) -> Result<Vec<u8>> {
    ensure!(bytes.len() <= limit, "release archive exceeds size limit");
    if target.format == ArchiveFormat::Zip {
        let expected = [target.executable, "LICENSE", "README.md"].map(|name| MemberSpec {
            name,
            kind: MemberKind::File,
            max_bytes: limit as u64,
            exact_bytes: None,
            unix_mode: Some(if name == target.executable {
                0o100755
            } else {
                0o100644
            }),
        });
        let mut archive = Archive::open(bytes, &expected, zip_limits(limit))?;
        let mut binary = Vec::new();
        archive.copy(target.executable, &mut binary)?;
        ensure!(
            !binary.is_empty(),
            "release kuru entry is not an executable"
        );
        // Validate every member's decoded CRC/size, including non-executable documentation.
        for name in ["LICENSE", "README.md"] {
            archive.copy(name, &mut std::io::sink())?;
        }
        return Ok(binary);
    }
    // Bound all expanded bytes before the tar parser can allocate for PAX or GNU metadata.
    let expanded = bounded(GzDecoder::new(bytes), limit, "expanded release archive")?;
    let mut archive = tar::Archive::new(expanded.as_slice());
    let mut seen = HashSet::new();
    let mut binary = None;
    // Raw entries expose extension headers, which are unnecessary in our USTAR format.
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        let name = entry.path_bytes().to_vec();
        ensure!(
            [b"kuru".as_slice(), b"LICENSE", b"README.md"].contains(&name.as_slice())
                && entry.header().entry_type().is_file()
                && entry.size() <= limit as u64,
            "release archive contains unsafe paths, links or oversized entries"
        );
        ensure!(
            seen.insert(name.clone()),
            "release archive must contain exactly one of each entry"
        );
        if name == b"kuru" {
            ensure!(
                entry.header().mode()? & 0o100 != 0 && entry.size() > 0,
                "release kuru entry is not an executable"
            );
            let expected_size = entry.size();
            let content = bounded(&mut entry, limit, "release executable")?;
            ensure!(
                content.len() as u64 == expected_size,
                "release executable is truncated"
            );
            binary = Some(content);
        }
    }
    binary.context("release archive must contain exactly one kuru executable")
}

fn destination_directory(path: &Path) -> Result<Directory> {
    let path = absolute(path)?;
    match Directory::open(&path, Privacy::Inherited, NameRetention::Movable) {
        Ok(directory) => Ok(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            drop(Directory::ensure_private(&path)?);
            Ok(Directory::open(
                &path,
                Privacy::Inherited,
                NameRetention::Movable,
            )?)
        }
        Err(error) => Err(error.into()),
    }
}

fn install_binary(binary: &[u8], destination: &Path, target: &Target) -> Result<PathBuf> {
    let destination = destination_directory(destination)?;
    let name = OsStr::new(target.executable);
    match destination.read(name) {
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error).context("destination must be a regular file, not a symlink, reparse point, hardlink or directory"),
    }
    #[cfg(windows)]
    let _lease = crate::update::installation_guard(&destination)?;
    let staging = crate::staging::Stage::create(&destination, ".kuru-install-")?;
    // Create and validate the protected root before any candidate content exists.
    // The ordinary payload retains the destination's executable access convention.
    let stage = Directory::open(staging.path(), Privacy::Inherited, NameRetention::Movable)?;
    let mut output = stage.create_new(name)?;
    output.write_all(binary)?;
    make_executable(&output)?;
    output.sync_all()?;
    if let Err(error) =
        destination.publish_file(&stage, name, &output, name, Publication::ReplaceRegular)
        && (error.phase != PublicationPhase::Uncertain
            || destination.verify(name, &output).is_err())
    {
        return Err(error.into());
    }
    drop(output);
    drop(stage);
    staging
        .finish()
        .context("verified executable was installed, but private staging cleanup failed")?;
    Ok(destination.path().join(name))
}

pub fn package(binary: &Path, target: &str, version: &str, output: &Path) -> Result<PathBuf> {
    package_with_docs(
        binary,
        target,
        version,
        output,
        include_bytes!("../../../LICENSE"),
        include_bytes!("../../../README.md"),
    )
}

fn package_with_docs(
    binary: &Path,
    target: &str,
    version: &str,
    output: &Path,
    license: &[u8],
    readme: &[u8],
) -> Result<PathBuf> {
    let binary = absolute(binary)?;
    let parent = Directory::open(
        binary.parent().context("binary has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let input_name = binary.file_name().context("binary has no filename")?;
    let mut input = open_build_input(&parent, input_name)
        .context("binary must be an existing executable file")?;
    let before = regular_file_info(&input)?;
    let name = archive_name(version, target)?;
    let output = destination_directory(output)?;
    let archive = output.path().join(&name);
    let checksum = output.path().join(format!("{name}.sha256"));
    for path in [&archive, &checksum] {
        match output.read(path.file_name().context("package output has no filename")?) {
            Ok(existing) => {
                ensure!(
                    regular_file_info(&existing)?.identity != before.identity,
                    "package output must not replace its input executable"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("package output must be a regular file, not a symlink, reparse point, hardlink or directory"),
        }
    }
    let target = crate::targets::find(target)?;
    let expanded_size = [before.len, license.len() as u64, readme.len() as u64]
        .into_iter()
        .try_fold(1024u64, |total, size| {
            size.checked_add(511)
                .and_then(|size| size.checked_div(512))
                .and_then(|blocks| blocks.checked_mul(512))
                .and_then(|padded| padded.checked_add(512))
                .and_then(|entry| total.checked_add(entry))
        });
    ensure!(
        expanded_size.is_some_and(|size| size <= MAX_ARCHIVE_BYTES as u64),
        "expanded release archive exceeds size limit"
    );
    // Build both artifacts privately. A read/compression/write failure must
    // not truncate an earlier release or publish a partial replacement.
    let staging = crate::staging::Stage::create(&output, ".kuru-package-")?;
    let candidate = staging.path().join(&name);
    let contents = [
        (
            target.executable,
            bounded(&mut input, MAX_ARCHIVE_BYTES, "release executable")?,
            0o755,
        ),
        ("LICENSE", license.to_vec(), 0o644),
        ("README.md", readme.to_vec(), 0o644),
    ];
    verify_build_snapshot(&parent, input_name, &mut input, before, &contents[0].1)?;
    match target.format {
        ArchiveFormat::TarGz => {
            let gzip = GzBuilder::new().mtime(0).write(
                staging.directory().create_new(OsStr::new(&name))?,
                Compression::default(),
            );
            let mut stream = tar::Builder::new(gzip);
            for (name, content, mode) in &contents {
                let mut header = tar::Header::new_ustar();
                header.set_size(content.len() as u64);
                header.set_mode(*mode);
                header.set_uid(0);
                header.set_gid(0);
                header.set_mtime(0);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                stream.append_data(&mut header, *name, content.as_slice())?;
            }
            stream.into_inner()?.finish()?.sync_all()?;
        }
        ArchiveFormat::Zip => {
            let members: Vec<_> = contents
                .iter()
                .map(|(name, content, mode)| WriteMember {
                    name,
                    kind: MemberKind::File,
                    bytes: content,
                    executable: mode & 0o111 != 0,
                })
                .collect();
            let bytes = zip::write(&members, zip_limits(MAX_ARCHIVE_BYTES))?;
            let mut file = staging.directory().create_new(OsStr::new(&name))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
    }
    ensure!(
        fs::metadata(&candidate)?.len() <= MAX_ARCHIVE_BYTES as u64,
        "compressed release archive exceeds size limit"
    );
    let mut output_checksum = staging.directory().create_new(OsStr::new("SHA256SUMS"))?;
    writeln!(
        output_checksum,
        "{}  {name}",
        digest(&fs::read(&candidate)?)
    )?;
    output_checksum.sync_all()?;
    drop(output_checksum);
    let stage = staging.directory();
    let archive_file = stage.read_write(OsStr::new(&name))?;
    output.publish_file(
        stage,
        OsStr::new(&name),
        &archive_file,
        OsStr::new(&name),
        Publication::ReplaceRegular,
    )?;
    let checksum_file = stage.read_write(OsStr::new("SHA256SUMS"))?;
    output.publish_file(
        stage,
        OsStr::new("SHA256SUMS"),
        &checksum_file,
        checksum.file_name().context("missing checksum filename")?,
        Publication::ReplaceRegular,
    )?;
    drop(archive_file);
    drop(checksum_file);
    staging.finish()?;
    Ok(archive)
}

#[cfg(test)]
mod tests;
