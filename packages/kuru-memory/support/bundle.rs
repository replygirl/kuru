//! Local build-input policy, shared with behavioral tests; never uses the network.
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_COMPRESSED: u64 = 64 * 1024 * 1024;
pub const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub upstream_commit: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub target: String,
    pub stem: String,
    pub format: String,
    pub executable_name: String,
    pub url: String,
    pub compressed_bytes: u64,
    pub archive_sha256: String,
    pub expanded_bytes: u64,
    pub executable_bytes: u64,
    pub executable_sha256: String,
    pub license_bytes: u64,
    pub license_sha256: String,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&read_checked(path, MAX_MANIFEST)?)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() as u64 <= MAX_MANIFEST,
            "Dolt manifest exceeds byte budget"
        );
        let manifest: Self = serde_json::from_slice(bytes).context("parse Dolt bundle manifest")?;
        ensure!(
            manifest.schema_version == 1,
            "unsupported Dolt manifest schema"
        );
        let numbers: Vec<_> = manifest.version.split('.').collect();
        ensure!(
            numbers.len() == 3
                && numbers.iter().all(|number| !number.is_empty()
                    && number.bytes().all(|byte| byte.is_ascii_digit())
                    && (number.len() == 1 || !number.starts_with('0'))),
            "Dolt version must be three canonical decimal numbers"
        );
        ensure!(
            hex(&manifest.upstream_commit, 40),
            "invalid Dolt upstream commit"
        );
        ensure!(
            manifest.assets.len() == 5,
            "Dolt manifest must cover the five supported targets"
        );
        let mut targets = HashSet::new();
        for asset in &manifest.assets {
            ensure!(targets.insert(&asset.target), "duplicate Dolt target");
            let stem = match asset.target.as_str() {
                "aarch64-apple-darwin" => "dolt-darwin-arm64",
                "x86_64-apple-darwin" => "dolt-darwin-amd64",
                "aarch64-unknown-linux-gnu" => "dolt-linux-arm64",
                "x86_64-unknown-linux-gnu" => "dolt-linux-amd64",
                "x86_64-pc-windows-msvc" => "dolt-windows-amd64",
                _ => anyhow::bail!("unsupported Dolt bundle target {}", asset.target),
            };
            ensure!(
                asset.stem == stem,
                "Dolt archive stem does not match target"
            );
            let (format, executable_name) = if asset.target == "x86_64-pc-windows-msvc" {
                ("zip", "dolt.exe")
            } else {
                ("tar.gz", "dolt")
            };
            ensure!(
                asset.format == format && asset.executable_name == executable_name,
                "Dolt archive format or executable does not match target"
            );
            ensure!(
                asset.url
                    == format!(
                        "https://github.com/dolthub/dolt/releases/download/v{}/{}.{}",
                        manifest.version, stem, format
                    ),
                "Dolt archive URL does not match pinned release"
            );
            ensure!(
                asset.compressed_bytes > 0 && asset.compressed_bytes <= MAX_COMPRESSED,
                "Dolt compressed size exceeds budget"
            );
            ensure!(
                asset.expanded_bytes > 0 && asset.expanded_bytes <= MAX_EXPANDED,
                "Dolt expanded size exceeds budget"
            );
            ensure!(
                asset.executable_bytes > 0
                    && asset.license_bytes > 0
                    && asset
                        .executable_bytes
                        .checked_add(asset.license_bytes)
                        .is_some_and(|size| if format == "zip" {
                            size == asset.expanded_bytes
                        } else {
                            size < asset.expanded_bytes
                        }),
                "Dolt payload sizes exceed expanded archive"
            );
            for digest in [
                &asset.archive_sha256,
                &asset.executable_sha256,
                &asset.license_sha256,
            ] {
                ensure!(hex(digest, 64), "invalid Dolt SHA-256 digest");
            }
        }
        Ok(manifest)
    }

    pub fn select(&self, target: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|asset| asset.target == target)
            .with_context(|| {
                format!("no bundled Dolt for Cargo TARGET {target}; no host fallback is allowed")
            })
    }

    pub fn catalog(&self, target: &str) -> Result<String> {
        let selected = self.select(target)?;
        let render = |asset: &Asset| {
            format!(
                "Asset {{ target: {:?}, stem: {:?}, format: {:?}, executable_name: {:?}, compressed_bytes: {}, archive_sha256: {:?}, expanded_bytes: {}, executable_bytes: {}, executable_sha256: {:?}, license_bytes: {}, license_sha256: {:?} }}",
                asset.target,
                asset.stem,
                asset.format,
                asset.executable_name,
                asset.compressed_bytes,
                asset.archive_sha256,
                asset.expanded_bytes,
                asset.executable_bytes,
                asset.executable_sha256,
                asset.license_bytes,
                asset.license_sha256
            )
        };
        Ok(format!(
            "pub const DOLT_VERSION: &str = {:?};\npub(crate) const BUNDLED_ASSET: Asset<'static> = {};\n#[cfg(test)]\npub(crate) const ASSETS: [Asset<'static>; 5] = [{}];\n",
            self.version,
            render(selected),
            self.assets.iter().map(render).collect::<Vec<_>>().join(",")
        ))
    }
}

pub fn bundle_directory(package: &Path, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(directory) = override_path {
        ensure!(
            directory.is_absolute(),
            "KURU_DOLT_BUNDLE_DIR must be absolute"
        );
        return Ok(directory.to_path_buf());
    }
    Ok(package
        .parent()
        .and_then(Path::parent)
        .context("memory package needs its workspace parent")?
        .join("target/kuru-bundles"))
}

pub fn prepared_archive(directory: &Path, asset: &Asset) -> PathBuf {
    directory.join(format!("{}.archive", asset.archive_sha256))
}

pub fn verified_archive(path: &Path, asset: &Asset) -> Result<Vec<u8>> {
    ensure!(
        asset.compressed_bytes > 0 && asset.compressed_bytes <= MAX_COMPRESSED,
        "Dolt compressed size exceeds budget"
    );
    let bytes = read_checked(path, asset.compressed_bytes)?;
    ensure!(
        bytes.len() as u64 == asset.compressed_bytes,
        "prepared Dolt archive size does not match target"
    );
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        digest == asset.archive_sha256,
        "prepared Dolt archive checksum mismatch"
    );
    Ok(bytes)
}

fn read_checked(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "Dolt build input path must be absolute");
    let parent = Directory::open(
        path.parent().context("build input has no parent")?,
        Privacy::Inherited,
        NameRetention::Pinned,
    )?;
    let name = path.file_name().context("build input has no name")?;
    let mut file = parent.read(name)?;
    ensure!(
        regular_file_info(&file)?.len <= limit,
        "Dolt build input must be a bounded regular file without links"
    );
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    parent.verify(name, &file)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Dolt build input changed while reading"
    );
    Ok(bytes)
}
