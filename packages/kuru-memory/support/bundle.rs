//! Local build-input policy, shared with behavioral tests; never uses the network.
use anyhow::{Context, Result, bail, ensure};
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
const MAX_SOURCE_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_TOOLCHAIN_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_NOTICE: u64 = 1024 * 1024;
/// Explicit placeholder for a built asset whose linux-x64 pins do not exist yet.
pub const UNPINNED: &str = "unpinned";
const RECIPES: [&str; 1] = ["dolt-cgo-llvm-mingw-icu-stub/1"];
const BUILD_HOST: &str = "linux-x64";
const DOLT_MODULE: &str = "github.com/dolthub/dolt/go";
const ICU_RELEASES: &str = "https://github.com/unicode-org/icu/releases/download/";
const GO_DOWNLOADS: &str = "https://dl.google.com/go/";
const LLVM_MINGW_RELEASES: &str = "https://github.com/mstorsjo/llvm-mingw/releases/download/";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub upstream_commit: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    Upstream,
    Built,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub target: String,
    pub stem: String,
    pub format: String,
    pub executable_name: String,
    pub provenance: Provenance,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub build: Option<Build>,
    pub compressed_bytes: Option<u64>,
    pub archive_sha256: String,
    pub expanded_bytes: Option<u64>,
    pub executable_bytes: Option<u64>,
    pub executable_sha256: String,
    pub license_bytes: u64,
    pub license_sha256: String,
    #[serde(default)]
    pub notices: Option<Vec<Notice>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub recipe: String,
    pub host: String,
    pub goos: String,
    pub goarch: String,
    pub tags: Vec<String>,
    pub sources: Sources,
    pub toolchain: Toolchain,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub dolt: DoltSource,
    pub icu: IcuSource,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoltSource {
    pub module: String,
    pub version: String,
    pub sum: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcuSource {
    pub version: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub go: GoToolchain,
    pub llvm_mingw: LlvmMingw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoToolchain {
    pub version: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlvmMingw {
    pub version: String,
    pub clang_version: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSource {
    Icu,
    LlvmMingw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub name: String,
    pub from: NoticeSource,
    pub path: String,
    pub bytes: Option<u64>,
    pub sha256: String,
}

/// The exact archive identity of a pinned asset.
#[derive(Clone, Copy, Debug)]
pub struct Pins<'a> {
    pub compressed_bytes: u64,
    pub archive_sha256: &'a str,
    pub expanded_bytes: u64,
    pub executable_bytes: u64,
    pub executable_sha256: &'a str,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn go_sum(value: &str) -> bool {
    value.strip_prefix("h1:").is_some_and(|encoded| {
        encoded.len() == 44
            && encoded.ends_with('=')
            && encoded[..43]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    })
}

fn notice_name(value: &str) -> bool {
    value.strip_prefix("LICENSE-").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 63
            && rest.as_bytes()[0].is_ascii_alphanumeric()
            && rest.bytes().all(|byte| {
                byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
            })
    })
}

fn relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn target_stem(target: &str) -> Result<&'static str> {
    Ok(match target {
        "aarch64-apple-darwin" => "dolt-darwin-arm64",
        "x86_64-apple-darwin" => "dolt-darwin-amd64",
        "aarch64-unknown-linux-gnu" => "dolt-linux-arm64",
        "x86_64-unknown-linux-gnu" => "dolt-linux-amd64",
        "x86_64-pc-windows-msvc" => "dolt-windows-amd64",
        "aarch64-pc-windows-msvc" => "dolt-windows-arm64",
        _ => bail!("unsupported Dolt bundle target {target}"),
    })
}

fn unpinned_message(target: &str) -> String {
    format!(
        "Dolt engine for {target} is built from source and not yet pinned: run `mise run //packages/kuru-memory:bundle:build -- --target {target} --print-pins` on linux-x64 and commit the pins"
    )
}

impl Asset {
    /// Return this asset's pinned archive identity. Upstream assets are always
    /// pinned; a built asset is refused until the linux-x64 pins are committed.
    pub fn pins(&self) -> Result<Pins<'_>> {
        match (
            self.compressed_bytes,
            self.expanded_bytes,
            self.executable_bytes,
        ) {
            (Some(compressed_bytes), Some(expanded_bytes), Some(executable_bytes))
                if self.archive_sha256 != UNPINNED && self.executable_sha256 != UNPINNED =>
            {
                Ok(Pins {
                    compressed_bytes,
                    archive_sha256: &self.archive_sha256,
                    expanded_bytes,
                    executable_bytes,
                    executable_sha256: &self.executable_sha256,
                })
            }
            _ => bail!("{}", unpinned_message(&self.target)),
        }
    }

    pub fn notices(&self) -> &[Notice] {
        self.notices.as_deref().unwrap_or_default()
    }

    fn fully_pinned(&self) -> bool {
        self.pins().is_ok()
            && self
                .notices()
                .iter()
                .all(|notice| notice.bytes.is_some() && notice.sha256 != UNPINNED)
    }

    fn validate(&self, manifest: &Manifest) -> Result<()> {
        let stem = target_stem(&self.target)?;
        ensure!(self.stem == stem, "Dolt archive stem does not match target");
        let (format, executable_name) = if self.target.ends_with("-pc-windows-msvc") {
            ("zip", "dolt.exe")
        } else {
            ("tar.gz", "dolt")
        };
        ensure!(
            self.format == format && self.executable_name == executable_name,
            "Dolt archive format or executable does not match target"
        );
        ensure!(
            self.license_bytes > 0 && hex(&self.license_sha256, 64),
            "invalid Dolt license pin"
        );
        let archive_fields = [
            self.compressed_bytes.is_some(),
            self.expanded_bytes.is_some(),
            self.executable_bytes.is_some(),
            self.archive_sha256 != UNPINNED,
            self.executable_sha256 != UNPINNED,
        ];
        let pinned = archive_fields.iter().all(|field| *field);
        ensure!(
            pinned || archive_fields.iter().all(|field| !*field),
            "Dolt archive pins must be all pinned or all unpinned"
        );
        match self.provenance {
            Provenance::Upstream => {
                ensure!(pinned, "upstream Dolt assets can never be unpinned");
                ensure!(
                    self.build.is_none() && self.notices.is_none(),
                    "upstream Dolt assets must not declare a build or notices"
                );
                ensure!(
                    self.url.as_deref()
                        == Some(
                            format!(
                                "https://github.com/dolthub/dolt/releases/download/v{}/{}.{}",
                                manifest.version, stem, format
                            )
                            .as_str()
                        ),
                    "Dolt archive URL does not match pinned release"
                );
            }
            Provenance::Built => {
                ensure!(
                    self.url.is_none(),
                    "built Dolt assets must not declare a URL"
                );
                let build = self
                    .build
                    .as_ref()
                    .context("built Dolt assets must declare their build")?;
                build.validate(&self.target, manifest)?;
                self.validate_notices(pinned)?;
                for upstream in manifest
                    .assets
                    .iter()
                    .filter(|asset| asset.provenance == Provenance::Upstream)
                {
                    ensure!(
                        upstream.license_sha256 == self.license_sha256
                            && upstream.license_bytes == self.license_bytes,
                        "built Dolt LICENSES must equal the upstream Godeps/LICENSES pin"
                    );
                }
            }
        }
        if let Ok(pins) = self.pins() {
            ensure!(
                pins.compressed_bytes > 0 && pins.compressed_bytes <= MAX_COMPRESSED,
                "Dolt compressed size exceeds budget"
            );
            ensure!(
                pins.expanded_bytes > 0 && pins.expanded_bytes <= MAX_EXPANDED,
                "Dolt expanded size exceeds budget"
            );
            let notices = self
                .notices()
                .iter()
                .try_fold(0_u64, |total, notice| total.checked_add(notice.bytes?));
            ensure!(
                pins.executable_bytes > 0
                    && pins
                        .executable_bytes
                        .checked_add(self.license_bytes)
                        .and_then(|size| size.checked_add(notices?))
                        .is_some_and(|size| if format == "zip" {
                            size == pins.expanded_bytes
                        } else {
                            size < pins.expanded_bytes
                        }),
                "Dolt payload sizes exceed expanded archive"
            );
            for digest in [pins.archive_sha256, pins.executable_sha256] {
                ensure!(hex(digest, 64), "invalid Dolt SHA-256 digest");
            }
        }
        Ok(())
    }

    fn validate_notices(&self, archive_pinned: bool) -> Result<()> {
        let notices = self
            .notices
            .as_deref()
            .filter(|notices| !notices.is_empty() && notices.len() <= 16)
            .context("built Dolt assets must declare their third-party notices")?;
        let mut names = HashSet::new();
        for notice in notices {
            ensure!(
                notice_name(&notice.name) && names.insert(notice.name.as_str()),
                "invalid or duplicate Dolt notice name"
            );
            ensure!(
                relative_path(&notice.path)
                    && (notice.from != NoticeSource::Icu || notice.path.starts_with("icu/")),
                "invalid Dolt notice path"
            );
            match (notice.bytes, notice.sha256.as_str()) {
                (None, UNPINNED) => ensure!(
                    !archive_pinned,
                    "a pinned Dolt archive requires pinned notices"
                ),
                (Some(bytes), digest) => ensure!(
                    bytes > 0 && bytes <= MAX_NOTICE && hex(digest, 64),
                    "invalid Dolt notice pin"
                ),
                (None, _) => bail!("Dolt notice pins must be both pinned or both unpinned"),
            }
        }
        Ok(())
    }
}

impl Build {
    fn validate(&self, target: &str, manifest: &Manifest) -> Result<()> {
        ensure!(
            RECIPES.contains(&self.recipe.as_str()),
            "unknown Dolt build recipe"
        );
        ensure!(self.host == BUILD_HOST, "Dolt builds run only on linux-x64");
        let (goos, goarch) = match target {
            "aarch64-pc-windows-msvc" => ("windows", "arm64"),
            _ => bail!("no Dolt source build is defined for {target}"),
        };
        ensure!(
            self.goos == goos && self.goarch == goarch,
            "Dolt build platform does not match target"
        );
        ensure!(
            self.tags == ["icu_static", "timetzdata"],
            "Dolt build tags do not match the recipe"
        );
        let dolt = &self.sources.dolt;
        ensure!(dolt.module == DOLT_MODULE, "unexpected Dolt Go module");
        ensure!(
            dolt.version.starts_with('v')
                && dolt.version.len() <= 64
                && dolt
                    .version
                    .ends_with(&format!("-{}", &manifest.upstream_commit[..12])),
            "Dolt module version does not pin the upstream commit"
        );
        ensure!(go_sum(&dolt.sum), "invalid Dolt module checksum");
        let icu = &self.sources.icu;
        ensure!(
            !icu.version.is_empty()
                && icu.version.len() <= 16
                && icu
                    .version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
                && icu.url.starts_with(ICU_RELEASES)
                && icu.bytes > 0
                && icu.bytes <= MAX_SOURCE_ARCHIVE
                && hex(&icu.sha256, 64),
            "invalid ICU source pin"
        );
        let go = &self.toolchain.go;
        ensure!(
            go.version.starts_with("go") && go.url.starts_with(GO_DOWNLOADS) && hex(&go.sha256, 64),
            "invalid Go toolchain pin"
        );
        let llvm = &self.toolchain.llvm_mingw;
        ensure!(
            !llvm.version.is_empty()
                && llvm.version.bytes().all(|byte| byte.is_ascii_digit())
                && !llvm.clang_version.is_empty()
                && llvm
                    .clang_version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
                && llvm
                    .url
                    .starts_with(&format!("{LLVM_MINGW_RELEASES}{}/", llvm.version))
                && llvm.bytes > 0
                && llvm.bytes <= MAX_TOOLCHAIN_ARCHIVE
                && hex(&llvm.sha256, 64),
            "invalid llvm-mingw toolchain pin"
        );
        Ok(())
    }
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
            manifest.schema_version == 2,
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
            manifest.assets.len() == 6,
            "Dolt manifest must cover the six known targets"
        );
        let mut targets = HashSet::new();
        for asset in &manifest.assets {
            ensure!(targets.insert(&asset.target), "duplicate Dolt target");
            asset.validate(&manifest)?;
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
        ensure!(selected.fully_pinned(), "{}", unpinned_message(target));
        let render = |asset: &Asset| -> Result<String> {
            let pins = asset.pins()?;
            let notices = asset
                .notices()
                .iter()
                .map(|notice| {
                    Ok(format!(
                        "Notice {{ name: {:?}, bytes: {}, sha256: {:?} }}",
                        notice.name,
                        notice.bytes.context("unpinned Dolt notice")?,
                        notice.sha256
                    ))
                })
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            Ok(format!(
                "Asset {{ target: {:?}, stem: {:?}, format: {:?}, executable_name: {:?}, compressed_bytes: {}, archive_sha256: {:?}, expanded_bytes: {}, executable_bytes: {}, executable_sha256: {:?}, license_bytes: {}, license_sha256: {:?}, notices: &[{notices}] }}",
                asset.target,
                asset.stem,
                asset.format,
                asset.executable_name,
                pins.compressed_bytes,
                pins.archive_sha256,
                pins.expanded_bytes,
                pins.executable_bytes,
                pins.executable_sha256,
                asset.license_bytes,
                asset.license_sha256
            ))
        };
        // Only fully pinned assets can be represented in the runtime catalog.
        let pinned = self
            .assets
            .iter()
            .filter(|asset| asset.fully_pinned())
            .map(render)
            .collect::<Result<Vec<_>>>()?;
        Ok(format!(
            "pub const DOLT_VERSION: &str = {:?};\npub(crate) const BUNDLED_ASSET: Asset<'static> = {};\n#[cfg(test)]\npub(crate) const ASSETS: [Asset<'static>; {}] = [{}];\n",
            self.version,
            render(selected)?,
            pinned.len(),
            pinned.join(",")
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
        .join("target")
        .join("kuru-bundles"))
}

pub fn prepared_archive(directory: &Path, asset: &Asset) -> Result<PathBuf> {
    Ok(directory.join(format!("{}.archive", asset.pins()?.archive_sha256)))
}

pub fn verified_archive(path: &Path, asset: &Asset) -> Result<Vec<u8>> {
    let pins = asset.pins()?;
    ensure!(
        pins.compressed_bytes > 0 && pins.compressed_bytes <= MAX_COMPRESSED,
        "Dolt compressed size exceeds budget"
    );
    let bytes = read_checked(path, pins.compressed_bytes)?;
    ensure!(
        bytes.len() as u64 == pins.compressed_bytes,
        "prepared Dolt archive size does not match target"
    );
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        digest == pins.archive_sha256,
        "prepared Dolt archive checksum mismatch"
    );
    Ok(bytes)
}

fn read_checked(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "Dolt build input path must be absolute");
    let parent_path = path.parent().context("build input has no parent")?;
    let parent = Directory::open(parent_path, Privacy::Inherited, NameRetention::Pinned)
        .with_context(|| {
            format!(
                "open checked build-input directory {}",
                parent_path.display()
            )
        })?;
    let name = path.file_name().context("build input has no name")?;
    let mut file = parent
        .read(name)
        .with_context(|| format!("open checked build-input file {}", path.display()))?;
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
