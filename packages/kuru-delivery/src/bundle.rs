//! Verified, hash-addressed build inputs. This helper never executes an archive
//! or depends on the runtime that will eventually consume its bytes.

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{
    Directory as CheckedDirectory, NameRetention, Privacy, Publication, require_private,
    seal_private,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{File, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const LOCK_NAME: &str = ".prepare.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(180);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(15)];

#[cfg(test)]
#[path = "bundle/recovery_tests.rs"]
mod recovery_tests;

#[derive(Debug)]
pub struct PrepareOptions {
    pub manifest: PathBuf,
    pub target: String,
    pub bundle_dir: PathBuf,
    pub archive: Option<PathBuf>,
    pub offline: bool,
}

/// The schema v2 Dolt asset manifest. `kuru-memory` owns an independent
/// parser of the same file; both reject unknown fields at every level.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    schema_version: u32,
    pub(crate) version: String,
    pub(crate) upstream_commit: String,
    pub(crate) assets: Vec<ManifestAsset>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Provenance {
    Upstream,
    Built,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestAsset {
    pub(crate) target: String,
    pub(crate) stem: String,
    pub(crate) format: String,
    pub(crate) executable_name: String,
    pub(crate) provenance: Provenance,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    pub(crate) build: Option<Build>,
    pub(crate) compressed_bytes: Option<u64>,
    pub(crate) archive_sha256: String,
    pub(crate) expanded_bytes: Option<u64>,
    pub(crate) executable_bytes: Option<u64>,
    pub(crate) executable_sha256: String,
    pub(crate) license_bytes: u64,
    pub(crate) license_sha256: String,
    #[serde(default)]
    pub(crate) notices: Option<Vec<Notice>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Build {
    pub(crate) recipe: String,
    pub(crate) host: String,
    pub(crate) goos: String,
    pub(crate) goarch: String,
    pub(crate) tags: Vec<String>,
    pub(crate) sources: Sources,
    pub(crate) toolchain: Toolchain,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sources {
    pub(crate) dolt: DoltSource,
    pub(crate) icu: IcuSource,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DoltSource {
    pub(crate) module: String,
    pub(crate) version: String,
    pub(crate) sum: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IcuSource {
    pub(crate) version: String,
    pub(crate) url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Toolchain {
    pub(crate) go: GoToolchain,
    pub(crate) llvm_mingw: LlvmMingw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoToolchain {
    pub(crate) version: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LlvmMingw {
    pub(crate) version: String,
    pub(crate) clang_version: String,
    pub(crate) url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NoticeSource {
    Icu,
    LlvmMingw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Notice {
    pub(crate) name: String,
    pub(crate) from: NoticeSource,
    pub(crate) path: String,
    pub(crate) bytes: Option<u64>,
    pub(crate) sha256: String,
}

/// Explicit placeholder for built-asset pins that only linux-x64 CI can supply.
pub(crate) const UNPINNED: &str = "unpinned";
const RECIPES: [&str; 1] = ["dolt-cgo-llvm-mingw-icu-stub/1"];
pub(crate) const BUILD_HOST: &str = "linux-x64";
const DOLT_MODULE: &str = "github.com/dolthub/dolt/go";
const ICU_RELEASES: &str = "https://github.com/unicode-org/icu/releases/download/";
const GO_DOWNLOADS: &str = "https://go.dev/dl/";
const LLVM_MINGW_RELEASES: &str = "https://github.com/mstorsjo/llvm-mingw/releases/download/";
const MAX_SOURCE_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_TOOLCHAIN_ARCHIVE: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_NOTICE: u64 = 1024 * 1024;

/// The pinned archive identity used to fetch, import or reuse one prepared file.
#[derive(Debug)]
struct Asset {
    target: String,
    url: String,
    /// Built archives are never downloaded: they are built on linux-x64 and imported.
    built: bool,
    compressed_bytes: u64,
    archive_sha256: String,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
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

pub(crate) fn relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains('\\')
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(crate) fn unpinned_message(target: &str) -> String {
    format!(
        "Dolt engine for {target} is built from source and not yet pinned: run `mise run //packages/kuru-memory:bundle:build -- --target {target} --print-pins` on linux-x64 and commit the pins"
    )
}

impl ManifestAsset {
    /// Whether every archive-identity field is pinned (never mixed; see validation).
    pub(crate) fn archive_pinned(&self) -> bool {
        self.compressed_bytes.is_some() && self.archive_sha256 != UNPINNED
    }

    pub(crate) fn notices(&self) -> &[Notice] {
        self.notices.as_deref().unwrap_or_default()
    }

    fn validate(&self, manifest: &Manifest) -> Result<()> {
        ensure!(
            identifier(&self.target) && identifier(&self.stem),
            "invalid target or stem in bundle manifest"
        );
        ensure!(
            matches!(
                (self.format.as_str(), self.executable_name.as_str()),
                ("tar.gz", "dolt") | ("zip", "dolt.exe")
            ),
            "unsupported bundle format or executable name"
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
            "bundle manifest pins must be all pinned or all unpinned"
        );
        ensure!(
            self.license_bytes > 0 && hex(&self.license_sha256, 64),
            "invalid SHA-256 in bundle manifest"
        );
        match self.provenance {
            Provenance::Upstream => {
                ensure!(pinned, "upstream bundle assets can never be unpinned");
                ensure!(
                    self.build.is_none() && self.notices.is_none(),
                    "upstream bundle assets must not declare a build or notices"
                );
                let url = url::Url::parse(self.url.as_deref().unwrap_or_default())
                    .context("invalid bundle archive URL")?;
                ensure!(
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "bundle archive URL must be HTTPS without credentials, query or fragment"
                );
            }
            Provenance::Built => {
                ensure!(
                    self.url.is_none(),
                    "built bundle assets must not declare a URL"
                );
                self.build
                    .as_ref()
                    .context("built bundle assets must declare their build")?
                    .validate(&self.target, manifest)?;
                self.validate_notices(pinned)?;
                ensure!(
                    manifest
                        .assets
                        .iter()
                        .filter(|asset| asset.provenance == Provenance::Upstream)
                        .all(|asset| asset.license_sha256 == self.license_sha256
                            && asset.license_bytes == self.license_bytes),
                    "built LICENSES must equal the upstream Godeps/LICENSES pin"
                );
            }
        }
        if pinned {
            let (compressed, expanded, executable) = (
                self.compressed_bytes.unwrap_or_default(),
                self.expanded_bytes.unwrap_or_default(),
                self.executable_bytes.unwrap_or_default(),
            );
            ensure!(
                compressed > 0 && compressed <= MAX_ARCHIVE,
                "bundle archive size must be within 64 MiB"
            );
            let notices = self
                .notices()
                .iter()
                .try_fold(0_u64, |total, notice| total.checked_add(notice.bytes?));
            ensure!(
                expanded > 0
                    && expanded <= MAX_EXPANDED
                    && executable > 0
                    && executable
                        .checked_add(self.license_bytes)
                        .and_then(|size| size.checked_add(notices?))
                        .is_some_and(|size| {
                            size <= expanded
                                && (self.provenance == Provenance::Upstream || size == expanded)
                        }),
                "invalid bounded payload sizes in bundle manifest"
            );
            ensure!(
                hex(&self.archive_sha256, 64) && hex(&self.executable_sha256, 64),
                "invalid SHA-256 in bundle manifest"
            );
        }
        Ok(())
    }

    fn validate_notices(&self, archive_pinned: bool) -> Result<()> {
        let notices = self
            .notices
            .as_deref()
            .filter(|notices| !notices.is_empty() && notices.len() <= 16)
            .context("built bundle assets must declare their third-party notices")?;
        let mut names = HashSet::new();
        for notice in notices {
            ensure!(
                notice_name(&notice.name) && names.insert(notice.name.as_str()),
                "invalid or duplicate bundle notice name"
            );
            ensure!(
                relative_path(&notice.path)
                    && (notice.from != NoticeSource::Icu || notice.path.starts_with("icu/")),
                "invalid bundle notice path"
            );
            match (notice.bytes, notice.sha256.as_str()) {
                (None, UNPINNED) => ensure!(
                    !archive_pinned,
                    "a pinned bundle archive requires pinned notices"
                ),
                (Some(bytes), digest) => ensure!(
                    bytes > 0 && bytes <= MAX_NOTICE && hex(digest, 64),
                    "invalid bundle notice pin"
                ),
                (None, _) => bail!("bundle notice pins must be both pinned or both unpinned"),
            }
        }
        Ok(())
    }
}

impl Build {
    fn validate(&self, target: &str, manifest: &Manifest) -> Result<()> {
        ensure!(
            RECIPES.contains(&self.recipe.as_str()),
            "unknown bundle build recipe"
        );
        ensure!(
            self.host == BUILD_HOST,
            "bundle builds run only on linux-x64"
        );
        let (goos, goarch) = match target {
            "aarch64-pc-windows-msvc" => ("windows", "arm64"),
            _ => bail!("no bundle source build is defined for {target}"),
        };
        ensure!(
            self.goos == goos && self.goarch == goarch,
            "bundle build platform does not match target"
        );
        ensure!(
            self.tags == ["icu_static", "timetzdata"],
            "bundle build tags do not match the recipe"
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

/// Read and validate the complete manifest. Validation of another target's
/// entry is structural only: no input of that target is fetched or read.
pub(crate) fn load_manifest(path: &Path) -> Result<Manifest> {
    let (directory, name) = input_parent(path)?;
    let mut file = directory.read(&name, false)?;
    ensure!(
        file.metadata()?.len() <= MAX_MANIFEST,
        "bundle manifest exceeds 64 KiB"
    );
    let mut bytes = Vec::new();
    (&mut file).take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "bundle manifest exceeds 64 KiB"
    );
    directory.verify(&name, &file, false)?;
    parse_manifest(&bytes)
}

pub(crate) fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes).context("parse Dolt asset manifest")?;
    ensure!(
        manifest.schema_version == 2,
        "unsupported Dolt asset manifest schema"
    );
    crate::archive::checked_version(&manifest.version)?;
    ensure!(
        hex(&manifest.upstream_commit, 40),
        "invalid upstream commit in bundle manifest"
    );
    ensure!(
        !manifest.assets.is_empty() && manifest.assets.len() <= 32,
        "bundle manifest must contain 1–32 targets"
    );
    let mut targets = HashSet::new();
    for asset in &manifest.assets {
        ensure!(
            targets.insert(&asset.target),
            "duplicate bundle target in manifest"
        );
        asset.validate(&manifest)?;
    }
    Ok(manifest)
}

impl Manifest {
    pub(crate) fn select(&self, target: &str) -> Result<&ManifestAsset> {
        self.assets
            .iter()
            .find(|asset| asset.target == target)
            .with_context(|| {
                format!(
                    "bundle manifest has no archive for target {target}; host fallback is disabled"
                )
            })
    }
}

fn manifest(path: &Path, target: &str) -> Result<Asset> {
    let manifest = load_manifest(path)?;
    let asset = manifest.select(target)?;
    ensure!(asset.archive_pinned(), "{}", unpinned_message(target));
    Ok(Asset {
        target: asset.target.clone(),
        url: asset.url.clone().unwrap_or_default(),
        built: asset.provenance == Provenance::Built,
        compressed_bytes: asset.compressed_bytes.unwrap_or_default(),
        archive_sha256: asset.archive_sha256.clone(),
    })
}

/// Resolve the conventional default from the manifest's owning workspace. An
/// override is explicit and must be absolute; neither path depends on Cargo's
/// host compilation target.
pub fn bundle_directory(manifest: &Path, override_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_dir {
        ensure!(
            path.is_absolute(),
            "bundle directory override must be absolute"
        );
        return Ok(path);
    }
    let manifest = absolute(manifest)?;
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("manifest must live under packages/kuru-memory/support")?;
    ensure!(
        manifest.ends_with("packages/kuru-memory/support/dolt-assets.json"),
        "provide --bundle-dir for a manifest outside packages/kuru-memory/support"
    );
    Ok(workspace.join("target/kuru-bundles"))
}

pub async fn prepare(options: &PrepareOptions) -> Result<PathBuf> {
    ensure!(
        options.bundle_dir.is_absolute(),
        "bundle directory must be absolute"
    );
    let target = if options.target == "host" {
        crate::archive::host_target()?
    } else {
        &options.target
    };
    let asset = manifest(&options.manifest, target)?;
    prepare_asset(options, &asset, None).await
}

async fn prepare_asset(
    options: &PrepareOptions,
    asset: &Asset,
    client: Option<&reqwest::Client>,
) -> Result<PathBuf> {
    prepare_asset_with_policy(options, asset, client, DOWNLOAD_TIMEOUT, &RETRY_DELAYS).await
}

async fn prepare_asset_with_policy(
    options: &PrepareOptions,
    asset: &Asset,
    client: Option<&reqwest::Client>,
    download_budget: Duration,
    retry_delays: &[Duration; 2],
) -> Result<PathBuf> {
    let directory =
        Directory::open(&options.bundle_dir, true, true).context("open bundle cache directory")?;
    let lock = directory
        .lock()
        .await
        .context("acquire stable bundle cache lock")?;
    let name = format!("{}.archive", asset.archive_sha256);
    let destination = directory.path().join(&name);
    match directory.read(OsStr::new(&name), true) {
        Ok(mut file) => {
            validate(&mut file, asset)?;
            directory.verify(OsStr::new(&name), &file, true)?;
            return Ok(destination);
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => {
            return Err(error).context("existing prepared bundle is unsafe; it was preserved");
        }
    }
    ensure!(
        options.archive.is_some() || !asset.built,
        "engine for {target} is built from source: run `mise run //packages/kuru-memory:bundle:build -- --target {target}` on linux-x64, then import it with --archive",
        target = asset.target
    );
    ensure!(
        options.archive.is_some() || !options.offline,
        "prepared bundle is missing in offline mode; import the pinned archive with --archive"
    );
    // The platform creates the file with the native access needed to seal its
    // DACL. A generic tempfile handle has write access but not WRITE_DAC on Windows.
    let stage = crate::staging::Stage::create(&directory.native, ".bundle-")?;
    let staged_name = OsStr::new("archive");
    let mut staging = stage.directory().create_new(staged_name)?;
    if let Some(path) = &options.archive {
        let (source, name) = input_parent(path).context("open local archive directory")?;
        let mut file = source.read(&name, false).context("open local archive")?;
        ensure!(
            file.metadata()?.len() == asset.compressed_bytes,
            "bundle archive size mismatch"
        );
        let mut digest = Sha256::new();
        let mut size = 0;
        let mut buffer = [0_u8; 65536];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            size += read as u64;
            ensure!(
                size <= asset.compressed_bytes,
                "bundle archive exceeds pinned size"
            );
            digest.update(&buffer[..read]);
            staging.write_all(&buffer[..read])?;
        }
        verify_digest(size, &digest.finalize(), asset)?;
        source
            .verify(&name, &file, false)
            .context("verify local archive identity")?;
    } else {
        let default_client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_IDLE_TIMEOUT)
            .timeout(DOWNLOAD_TIMEOUT)
            .build()?;
        download(
            client.unwrap_or(&default_client),
            asset,
            &mut staging,
            download_budget,
            retry_delays,
        )
        .await?;
    }
    seal_private(&staging, false)?;
    staging.sync_all()?;
    directory
        .revalidate()
        .context("verify cache directory before publication")?;
    directory.verify(OsStr::new(LOCK_NAME), &lock, true)?;
    stage
        .directory()
        .verify(staged_name, &staging)
        .context("verify staged archive before publication")?;
    directory
        .native
        .publish_file(
            stage.directory(),
            staged_name,
            &staging,
            OsStr::new(&name),
            Publication::New,
        )
        .context("publish prepared bundle without replacement")?;
    directory.verify(OsStr::new(&name), &staging, true)?;
    drop(staging);
    stage.finish()?;
    Ok(destination)
}

async fn download(
    client: &reqwest::Client,
    asset: &Asset,
    output: &mut File,
    budget: Duration,
    retry_delays: &[Duration; 2],
) -> Result<()> {
    tokio::time::timeout(budget, async {
        for attempt in 0..=retry_delays.len() {
            if attempt != 0 {
                output.set_len(0)?;
                output.seek(SeekFrom::Start(0))?;
                tokio::time::sleep(retry_delays[attempt - 1]).await;
            }

            let response = match client.get(&asset.url).send().await {
                Ok(response) => response,
                Err(error) if attempt < retry_delays.len() && retryable_send(&error) => {
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "bundle archive GET failed for target {} on attempt {}/{}",
                            asset.target,
                            attempt + 1,
                            retry_delays.len() + 1
                        )
                    });
                }
            };
            if attempt < retry_delays.len()
                && matches!(response.status().as_u16(), 500 | 502 | 503 | 504)
                && !response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER)
            {
                // No error-body bytes enter staging, and no response or worker
                // survives cancellation during the bounded backoff.
                drop(response);
                continue;
            }
            let status = response.status();
            let retry_after_present = response
                .headers()
                .contains_key(reqwest::header::RETRY_AFTER);
            let mut response = response.error_for_status().with_context(|| {
                format!(
                    "bundle archive GET returned {status} for target {} on attempt {}/{} (Retry-After present: {retry_after_present})",
                            asset.target,
                            attempt + 1,
                            retry_delays.len() + 1
                )
            })?;
            ensure!(
                response
                    .content_length()
                    .is_none_or(|size| size == asset.compressed_bytes),
                "bundle archive size mismatch"
            );
            let mut size = 0_u64;
            let mut digest = Sha256::new();
            let mut retry_body = false;
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    // This callsite has accepted a successful response. Reqwest
                    // may wrap a transport body-frame error as Decode, so keep
                    // this retry boundary narrower than a global Decode policy.
                    Err(_) if attempt < retry_delays.len() => {
                        retry_body = true;
                        break;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!(
                                "bundle archive body read failed for target {} on attempt {}/{} after {size} bytes",
                                asset.target,
                                attempt + 1,
                                retry_delays.len() + 1
                            )
                        });
                    }
                };
                size += chunk.len() as u64;
                ensure!(
                    size <= asset.compressed_bytes,
                    "bundle archive exceeds pinned size"
                );
                digest.update(&chunk);
                // Synchronous bounded chunk writes have no worker that can outlive
                // staging or lock when the surrounding future is cancelled.
                output.write_all(&chunk)?;
            }
            if retry_body {
                // The next attempt truncates and rewinds the private stage, then
                // starts a fresh digest before repeating the immutable GET.
                continue;
            }
            verify_digest(size, &digest.finalize(), asset)?;
            return Ok(());
        }
        unreachable!("the configured GET loop returns after its terminal attempt")
    })
    .await
    .context("bundle archive download timed out")?
}

fn retryable_send(error: &reqwest::Error) -> bool {
    // Protocol decoding happens before a usable response exists. A body-frame
    // error is handled only at `Response::chunk`, where retry is deliberate.
    !error.is_decode() && (error.is_timeout() || error.is_connect())
}

fn verify_digest(size: u64, digest: &[u8], asset: &Asset) -> Result<()> {
    ensure!(
        size == asset.compressed_bytes,
        "bundle archive size mismatch"
    );
    let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    ensure!(
        digest == asset.archive_sha256,
        "bundle archive checksum mismatch; existing inputs were preserved"
    );
    Ok(())
}

fn validate(file: &mut File, asset: &Asset) -> Result<()> {
    ensure!(
        file.metadata()?.len() == asset.compressed_bytes,
        "prepared bundle size mismatch; corrupt cache was preserved"
    );
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        ensure!(
            size <= asset.compressed_bytes,
            "prepared bundle exceeds pinned size"
        );
        digest.update(&buffer[..read]);
    }
    verify_digest(size, &digest.finalize(), asset)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::ParentDir)),
        "bundle paths must not contain parent traversal"
    );
    Ok(path)
}

fn input_parent(path: &Path) -> Result<(Directory, std::ffi::OsString)> {
    let path = absolute(path)?;
    let name = path
        .file_name()
        .context("bundle input needs a filename")?
        .to_owned();
    let directory = Directory::open(
        path.parent().context("bundle input needs a parent")?,
        false,
        false,
    )?;
    Ok((directory, name))
}

struct Directory {
    native: CheckedDirectory,
    private: bool,
}
impl Directory {
    fn open(path: &Path, create: bool, owner_only: bool) -> Result<Self> {
        let path = absolute(path)?;
        let native = if create {
            ensure!(owner_only, "new bundle directories must be private");
            CheckedDirectory::ensure_private(&path)?
        } else {
            CheckedDirectory::open(
                &path,
                if owner_only {
                    Privacy::OwnerOnly
                } else {
                    Privacy::Inherited
                },
                NameRetention::Movable,
            )?
        };
        Ok(Self {
            native,
            private: owner_only,
        })
    }
    fn path(&self) -> &Path {
        self.native.path()
    }
    fn revalidate(&self) -> Result<()> {
        let current = Self::open(self.path(), false, self.private)?;
        ensure!(
            current.native.identity() == self.native.identity(),
            "bundle directory or ancestor was replaced"
        );
        Ok(())
    }
    fn read(&self, name: &OsStr, owner_only: bool) -> Result<File> {
        let file = self.native.read(name)?;
        if owner_only {
            require_private(&file)?;
        }
        Ok(file)
    }
    fn verify(&self, name: &OsStr, held: &File, owner_only: bool) -> Result<()> {
        if owner_only {
            require_private(held)?;
        }
        self.native.verify(name, held)?;
        Ok(())
    }
    async fn lock(&self) -> Result<File> {
        let pinned =
            CheckedDirectory::open(self.path(), Privacy::OwnerOnly, NameRetention::Pinned)?;
        let lock = pinned
            .lock_file(OsStr::new(LOCK_NAME))
            .context("create or open stable lock file")?;
        let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "timed out waiting for bundle preparation; held lock was preserved"
                    );
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => bail!("lock bundle preparation: {error}"),
            }
        }
        self.verify(OsStr::new(LOCK_NAME), &lock, true)
            .context("verify acquired stable lock identity")?;
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_platform::fs::regular_file_info;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TEST_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(25), Duration::from_millis(50)];

    pub(super) fn asset(bytes: &[u8]) -> Asset {
        Asset {
            target: "test-target".into(),
            url: String::new(),
            built: false,
            compressed_bytes: bytes.len() as u64,
            archive_sha256: crate::archive::digest(bytes),
        }
    }
    pub(super) fn options(path: &Path) -> PrepareOptions {
        PrepareOptions {
            manifest: path.join("unused-manifest"),
            target: "test-target".into(),
            bundle_dir: path.join("cache"),
            archive: None,
            offline: false,
        }
    }
    pub(super) fn assert_clean(path: &Path) {
        let names: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsStr::new(LOCK_NAME)]);
        let lock = File::options()
            .read(true)
            .write(true)
            .open(path.join(LOCK_NAME))
            .unwrap();
        lock.try_lock().unwrap();
    }

    #[tokio::test]
    async fn transient_http_500_repeats_the_same_get_and_publishes_verified_bytes() {
        let expected = b"verified immutable archive";
        let root = tempfile::tempdir().unwrap();
        let options = options(root.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut asset = asset(expected);
        asset.url = format!(
            "http://{}/immutable.archive",
            listener.local_addr().unwrap()
        );
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                for (status, body) in [
                    (500, b"untrusted transient error body".as_slice()),
                    (200, expected.as_slice()),
                ] {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        request.push(socket.read_u8().await.unwrap());
                        assert!(request.len() < 8192);
                    }
                    observed.lock().unwrap().push(request);
                    let mut response = format!(
                        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body);
                    let _ = socket.write_all(&response).await;
                }
            })
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            prepare_asset_with_policy(
                &options,
                &asset,
                Some(&client),
                Duration::from_secs(3),
                &TEST_RETRY_DELAYS,
            ),
        )
        .await;
        server.abort();
        let _ = server.await;
        let published = result.unwrap().expect("recover the transient HTTP 500");
        assert_eq!(fs::read(&published).unwrap(), expected);
        assert_eq!(
            published.file_name().unwrap(),
            OsStr::new(&format!("{}.archive", asset.archive_sha256))
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert!(requests[0].starts_with(b"GET /immutable.archive HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn streamed_download_validates_size_hash_and_status_before_publication() {
        let expected = b"verified network archive";
        for (body, status, succeeds) in [
            (expected.as_slice(), 200, true),
            (b"wrongxxx network archive".as_slice(), 200, false),
            (b"short".as_slice(), 200, false),
            (
                b"verified network archive with extra bytes".as_slice(),
                200,
                false,
            ),
            (expected.as_slice(), 503, false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                let mut response = format!("HTTP/1.1 {status} Fixture\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", body.len()).into_bytes();
                response.extend_from_slice(body);
                response.extend_from_slice(b"\r\n0\r\n\r\n");
                let _ = socket.write_all(&response).await;
            });
            let mut asset = asset(expected);
            asset.url = format!("http://{address}/archive");
            let options = options(root.path());
            // Only this private transport fixture supplies an HTTP client;
            // public prepare always validates HTTPS and creates https_only.
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let result = prepare_asset_with_policy(
                &options,
                &asset,
                Some(&client),
                Duration::from_secs(3),
                &TEST_RETRY_DELAYS,
            )
            .await;
            server.abort();
            let _ = server.await;
            if succeeds {
                assert_eq!(fs::read(result.unwrap()).unwrap(), expected);
            } else {
                assert!(result.is_err());
                assert_clean(&options.bundle_dir);
            }
        }
    }

    #[tokio::test]
    async fn cancelling_a_stalled_download_removes_stage_and_releases_stable_lock() {
        let root = tempfile::tempdir().unwrap();
        let options = options(root.path());
        let cache = options.bundle_dir.clone();
        let expected = b"verified network archive";
        let mut asset = asset(expected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        asset.url = format!("http://{}/archive", listener.local_addr().unwrap());
        let (release, released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() < 8192);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nveri\r\n").await.unwrap();
            tokio::time::timeout(Duration::from_secs(10), released)
                .await
                .unwrap()
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let download =
            tokio::spawn(async move { prepare_asset(&options, &asset, Some(&client)).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let staged = fs::read_dir(&cache)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .any(|entry| {
                        entry.file_name().to_string_lossy().starts_with(".bundle-")
                            && fs::metadata(entry.path().join("archive"))
                                .is_ok_and(|metadata| metadata.len() == 4)
                    });
                if staged {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fixture never observed the written partial download");
        let before = regular_file_info(&File::open(cache.join(LOCK_NAME)).unwrap())
            .unwrap()
            .identity;
        download.abort();
        assert!(download.await.unwrap_err().is_cancelled());
        assert_clean(&cache);
        let after = regular_file_info(&File::open(cache.join(LOCK_NAME)).unwrap())
            .unwrap()
            .identity;
        assert_eq!(before, after);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn replaced_lock_and_directory_are_rejected_without_adopting_new_objects() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cache");
        let directory = Directory::open(&path, true, true).unwrap();
        let held = directory.lock().await.unwrap();
        let old_lock = path.join("old-lock");
        #[cfg(windows)]
        {
            // The native pinned handle denies substitution instead of detecting
            // it afterward. Both preserve one actual lock and the original bytes.
            assert!(fs::rename(path.join(LOCK_NAME), &old_lock).is_err());
            assert!(fs::rename(&path, root.path().join("retired")).is_err());
            directory
                .verify(OsStr::new(LOCK_NAME), &held, true)
                .unwrap();
            assert!(
                File::options()
                    .read(true)
                    .write(true)
                    .open(path.join(LOCK_NAME))
                    .unwrap()
                    .try_lock()
                    .is_err()
            );
            drop(held);
            let replacement = directory.lock().await.unwrap();
            directory
                .verify(OsStr::new(LOCK_NAME), &replacement, true)
                .unwrap();
        }
        #[cfg(unix)]
        {
            fs::rename(path.join(LOCK_NAME), &old_lock).unwrap();
            let replacement = File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.join(LOCK_NAME))
                .unwrap();
            replacement
                .set_permissions(fs::Permissions::from_mode(0o600))
                .unwrap();
            replacement.try_lock().unwrap();
            assert!(
                directory
                    .verify(OsStr::new(LOCK_NAME), &held, true)
                    .is_err()
            );
            assert!(
                File::options()
                    .read(true)
                    .write(true)
                    .open(&old_lock)
                    .unwrap()
                    .try_lock()
                    .is_err()
            );
            let old_directory = root.path().join("retired");
            fs::rename(&path, &old_directory).unwrap();
            Directory::open(&path, true, true).unwrap();
            assert!(directory.revalidate().is_err());
            assert!(old_directory.join("old-lock").is_file());
        }
    }

    #[tokio::test]
    async fn upstream_targets_fetch_only_their_own_archive_beside_an_unpinned_built_entry() {
        let committed = include_bytes!("../../kuru-memory/support/dolt-assets.json");
        let parsed = parse_manifest(committed).unwrap();
        let built = parsed.select("aarch64-pc-windows-msvc").unwrap();
        assert_eq!(built.provenance, Provenance::Built);
        assert!(!built.archive_pinned());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("dolt-assets.json");
        fs::write(&path, committed).unwrap();
        assert!(
            manifest(&path, "aarch64-pc-windows-msvc")
                .unwrap_err()
                .to_string()
                .contains("not yet pinned")
        );
        // Every request made by the preparer's client reaches this recording
        // proxy, so a fetch of any built input (ICU, Go, llvm-mingw) is visible.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                let line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                let body = line.as_bytes().to_vec();
                observed.lock().unwrap().push(line);
                let mut response = format!(
                    "HTTP/1.1 200 Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(&body);
                let _ = socket.write_all(&response).await;
            }
        });
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(&proxy).unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let upstream: Vec<_> = parsed
            .assets
            .iter()
            .filter(|asset| asset.provenance == Provenance::Upstream)
            .map(|asset| asset.target.clone())
            .collect();
        assert_eq!(upstream.len(), 5);
        for target in &upstream {
            let mut asset = manifest(&path, target).unwrap();
            assert!(!asset.built);
            // The fixture answers with the request line; pin the asset to it so
            // the unchanged verified publication path runs end to end.
            asset.url = asset.url.replacen("https://", "http://", 1);
            let expected = format!("GET {} HTTP/1.1", asset.url);
            asset.compressed_bytes = expected.len() as u64;
            asset.archive_sha256 = crate::archive::digest(expected.as_bytes());
            let options = PrepareOptions {
                bundle_dir: root.path().join(format!("cache-{target}")),
                ..options(root.path())
            };
            let published = prepare_asset_with_policy(
                &options,
                &asset,
                Some(&client),
                Duration::from_secs(5),
                &TEST_RETRY_DELAYS,
            )
            .await
            .unwrap();
            assert_eq!(fs::read(published).unwrap(), expected.as_bytes());
        }
        server.abort();
        let _ = server.await;
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 5, "{requests:?}");
        for (request, target) in requests.iter().zip(&upstream) {
            let stem = parsed.select(target).unwrap().stem.clone();
            assert!(
                request.starts_with("GET http://github.com/dolthub/dolt/releases/download/v2.3.3/")
                    && request.contains(&stem),
                "{request}"
            );
            for built_input in [
                "icu",
                "llvm-mingw",
                "go.dev",
                "proxy.golang",
                "windows-arm64",
            ] {
                assert!(!request.contains(built_input), "{request}");
            }
        }
    }
}
