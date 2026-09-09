//! Reproducible release archives and bounded, atomic installation.

use anyhow::{Context, Result, ensure};
use flate2::{Compression, GzBuilder, read::GzDecoder};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
];
const MAX_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

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
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(TARGETS[0]),
        ("macos", "x86_64") => Ok(TARGETS[1]),
        ("linux", "aarch64") => Ok(TARGETS[2]),
        ("linux", "x86_64") => Ok(TARGETS[3]),
        _ => anyhow::bail!("unsupported platform; build from source with Rust"),
    }
}

pub fn archive_name(version: &str, target: &str) -> Result<String> {
    let version = checked_version(version)?;
    ensure!(
        TARGETS.contains(&target),
        "unsupported platform; build from source with Rust"
    );
    Ok(format!("kuru-{version}-{target}.tar.gz"))
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
    if let Ok(mut base) = url::Url::parse(base) {
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
        bounded(
            File::open(Path::new(base).join(name))?,
            limit,
            "release asset",
        )
    }
}

fn bounded(reader: impl Read, limit: usize, description: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "{description} exceeds size limit");
    Ok(bytes)
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
    let name = archive_name(version, target)?;
    let expected = expected_digest(&read_asset(base, "SHA256SUMS", 64 * 1024).await?, &name)?;
    let bytes = read_asset(base, &name, MAX_ARCHIVE_BYTES).await?;
    ensure!(
        digest(&bytes) == expected,
        "release archive checksum mismatch; existing executable unchanged"
    );
    install_archive(&bytes, destination, MAX_ARCHIVE_BYTES)
}

fn install_archive(bytes: &[u8], destination: &Path, limit: usize) -> Result<PathBuf> {
    fs::create_dir_all(destination)?;
    let executable = destination.join("kuru");
    if let Ok(metadata) = fs::symlink_metadata(&executable) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "destination kuru must be a regular file, not a symlink or directory"
        );
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
    let binary = binary.context("release archive must contain exactly one kuru executable")?;
    let staging = tempfile::Builder::new()
        .prefix(".kuru-install-")
        .tempdir_in(destination)?;
    let candidate = staging.path().join("kuru");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&candidate)?;
    output.write_all(&binary)?;
    output.set_permissions(fs::Permissions::from_mode(0o755))?;
    output.sync_all()?;
    fs::rename(candidate, &executable)?;
    Ok(executable)
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
    let metadata =
        fs::symlink_metadata(binary).context("binary must be an existing executable file")?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.permissions().mode() & 0o111 != 0,
        "binary must be an existing executable file"
    );
    let name = archive_name(version, target)?;
    fs::create_dir_all(output)?;
    let archive = output.join(&name);
    let checksum = output.join(format!("{name}.sha256"));
    for path in [&archive, &checksum] {
        match fs::symlink_metadata(path) {
            Ok(existing) => {
                ensure!(
                    existing.is_file() && !existing.file_type().is_symlink(),
                    "package output must be a regular file, not a symlink or directory"
                );
                ensure!(
                    (existing.dev(), existing.ino()) != (metadata.dev(), metadata.ino()),
                    "package output must not replace its input executable"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect package output"),
        }
    }
    let expanded_size = [metadata.len(), license.len() as u64, readme.len() as u64]
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
    let staging = tempfile::Builder::new()
        .prefix(".kuru-package-")
        .tempdir_in(output)?;
    let candidate = staging.path().join(&name);
    let candidate_checksum = staging.path().join("SHA256SUMS");
    let gzip = GzBuilder::new()
        .mtime(0)
        .write(File::create(&candidate)?, Compression::default());
    let mut stream = tar::Builder::new(gzip);
    for (name, content, mode) in [
        (
            "kuru",
            bounded(File::open(binary)?, MAX_ARCHIVE_BYTES, "release executable")?,
            0o755,
        ),
        ("LICENSE", license.to_vec(), 0o644),
        ("README.md", readme.to_vec(), 0o644),
    ] {
        let mut header = tar::Header::new_ustar();
        header.set_size(content.len() as u64);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        stream.append_data(&mut header, name, content.as_slice())?;
    }
    stream.into_inner()?.finish()?.sync_all()?;
    let mut output_checksum = File::create(&candidate_checksum)?;
    writeln!(
        output_checksum,
        "{}  {name}",
        digest(&fs::read(&candidate)?)
    )?;
    output_checksum.sync_all()?;
    fs::rename(candidate, &archive)?;
    fs::rename(candidate_checksum, checksum)?;
    Ok(archive)
}

#[cfg(test)]
mod tests;
