//! Native post-publication acceptance for the exact Windows release.

use crate::{archive, command, mise_isolation, release, targets};
use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

const REPOSITORY: &str = "replygirl/kuru";
const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";
const COMMAND_DEADLINE: Duration = Duration::from_secs(180);
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const RECEIPT_LIMIT: usize = 64 * 1024;
const MISE_CONFIG: &str = r#"[settings]
use_versions_host=false
use_versions_host_track=false
netrc=false
[settings.github]
gh_cli_tokens=false
use_git_credentials=false
credential_command=""
oauth_client_id=""
"#;

pub struct Options {
    pub version: String,
    pub expected_sha: String,
    pub mise: PathBuf,
    pub manifest: PathBuf,
    pub evidence: PathBuf,
    pub run_url: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishedRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct Object {
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct TagReference {
    #[serde(rename = "ref")]
    name: String,
    object: Object,
}

#[derive(Debug, Deserialize)]
struct TagObject {
    object: Object,
}

#[derive(Debug, Deserialize)]
struct EngineManifest {
    schema_version: u32,
    version: String,
    assets: Vec<EngineAsset>,
}

#[derive(Debug, Deserialize)]
struct EngineAsset {
    target: String,
    executable_bytes: u64,
    executable_sha256: String,
    license_bytes: u64,
    license_sha256: String,
}

#[derive(Debug, Serialize)]
struct EngineEvidence {
    version: String,
    executable_sha256: String,
    license_sha256: String,
}

#[derive(Debug, Serialize)]
struct CommandEvidence {
    name: &'static str,
    status: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MiseSettings {
    #[serde(default)]
    url_replacements: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct KuruConfig<'a> {
    memory: MemoryConfig<'a>,
}

#[derive(Serialize)]
struct MemoryConfig<'a> {
    offline: bool,
    cache_dir: &'a Path,
}

#[derive(Debug, Serialize)]
struct Receipt {
    schema_version: u32,
    runner_os: &'static str,
    runner_arch: &'static str,
    repository: &'static str,
    release_url: String,
    run_url: String,
    version: String,
    commit: String,
    checksum_manifest_sha256: String,
    archive_sha256: String,
    executable_sha256: String,
    installed_sha256: String,
    mise_version: String,
    isolated_config_count: usize,
    commands: Vec<CommandEvidence>,
    session: String,
    first_revision: String,
    second_revision: String,
    engine: EngineEvidence,
    cleanup_confirmed: bool,
}

struct PublicGitHub {
    client: Client,
}

impl PublicGitHub {
    fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(Policy::limited(5))
                .timeout(Duration::from_secs(60))
                .user_agent("kuru-published-windows-verifier")
                .build()?,
        })
    }

    async fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let url = Url::parse(url)?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "published release URL must be credential-free HTTPS"
        );
        let mut response = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?
            .error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= limit as u64),
            "published response exceeds size limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "published response exceeds size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let bytes = self
            .get(
                &format!("https://api.github.com/repos/{REPOSITORY}/{path}"),
                METADATA_LIMIT,
            )
            .await?;
        serde_json::from_slice(&bytes).context("invalid GitHub release metadata")
    }

    async fn release(&self, version: &str) -> Result<PublishedRelease> {
        self.json(&format!("releases/tags/v{version}")).await
    }

    async fn tag_commit(&self, version: &str) -> Result<String> {
        let reference: TagReference = self.json(&format!("git/ref/tags/v{version}")).await?;
        ensure!(
            reference.name == format!("refs/tags/v{version}"),
            "GitHub returned a different tag"
        );
        let mut object = reference.object;
        let mut seen = HashSet::new();
        for _ in 0..4 {
            release::checked_sha(&object.sha)?;
            ensure!(
                seen.insert(object.sha.clone()),
                "release tag contains a cycle"
            );
            match object.kind.as_str() {
                "commit" => return Ok(object.sha),
                "tag" => {
                    object = self
                        .json::<TagObject>(&format!("git/tags/{}", object.sha))
                        .await?
                        .object;
                }
                _ => bail!("release tag does not resolve to a commit"),
            }
        }
        bail!("release tag indirection exceeds limit")
    }
}

fn expected_assets(version: &str) -> Result<BTreeSet<String>> {
    let mut names = targets::CATALOG
        .iter()
        .map(|target| archive::archive_name(version, target.triple))
        .collect::<Result<BTreeSet<_>>>()?;
    for target in &targets::CATALOG {
        names.insert(crate::shell_support::archive_name(version, target.triple)?);
    }
    names.insert("SHA256SUMS".to_owned());
    Ok(names)
}

fn validate_release(release: &PublishedRelease, version: &str) -> Result<()> {
    ensure!(
        release.tag_name == format!("v{version}") && !release.draft && !release.prerelease,
        "published release identity or state differs from the expected stable tag"
    );
    let expected = expected_assets(version)?;
    let actual: BTreeSet<_> = release
        .assets
        .iter()
        .map(|asset| asset.name.clone())
        .collect();
    ensure!(
        actual.len() == release.assets.len() && actual == expected,
        "published release asset inventory is incomplete, duplicated, or unexpected"
    );
    for asset in &release.assets {
        ensure!(
            asset.browser_download_url
                == format!(
                    "https://github.com/{REPOSITORY}/releases/download/v{version}/{}",
                    asset.name
                ),
            "published release asset URL differs from its immutable version path"
        );
    }
    Ok(())
}

fn parse_checksums(bytes: &[u8], version: &str) -> Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(bytes).context("SHA256SUMS is not UTF-8")?;
    let expected = expected_assets(version)?
        .into_iter()
        .filter(|name| name != "SHA256SUMS")
        .collect::<BTreeSet<_>>();
    let mut checksums = BTreeMap::new();
    for line in text.lines() {
        let (hash, name) = line
            .split_once("  ")
            .context("SHA256SUMS has an invalid entry")?;
        ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && expected.contains(name)
                && checksums.insert(name.to_owned(), hash.to_owned()).is_none(),
            "SHA256SUMS inventory or digest is invalid"
        );
    }
    ensure!(
        checksums.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "SHA256SUMS inventory is incomplete"
    );
    Ok(checksums)
}

fn validate_asset_digests(
    release: &PublishedRelease,
    checksums: &BTreeMap<String, String>,
    manifest_digest: &str,
) -> Result<()> {
    for asset in &release.assets {
        let expected = if asset.name == "SHA256SUMS" {
            manifest_digest
        } else {
            checksums
                .get(&asset.name)
                .context("release asset is absent from SHA256SUMS")?
        };
        ensure!(
            asset.digest.as_deref() == Some(&format!("sha256:{expected}")),
            "GitHub asset digest differs from downloaded publication"
        );
    }
    Ok(())
}

/// The published stable release immediately preceding a release candidate,
/// with its checksum manifest and Windows ZIP already authenticated. Kuru's
/// upgrade contract covers only this release: its own updater is what real
/// installations run against the candidate.
pub struct PreviousWindowsRelease {
    pub version: String,
    pub manifest: Vec<u8>,
    pub archive: Vec<u8>,
}

/// Resolve and download the release preceding `candidate` from public GitHub.
///
/// Nothing is pinned: the release is selected at run time by
/// [`select_previous`]. Both assets are read over HTTPS from their immutable
/// version paths, and no bytes are returned until the ZIP matches that
/// release's own `SHA256SUMS` and both match GitHub's asset digests.
pub async fn previous_windows_release(candidate: &str) -> Result<PreviousWindowsRelease> {
    let candidate = candidate.parse::<release::Version>()?;
    let github = PublicGitHub::new()?;
    // The unpaginated listing holds GitHub's 30 most recently created
    // releases, which always include the latest ones.
    let releases: Vec<PublishedRelease> = github.json("releases").await?;
    let version = select_previous(&releases, candidate)?.to_string();
    let listed = releases
        .iter()
        .find(|release| release.tag_name == format!("v{version}"))
        .context("selected previous release is absent from its listing")?;
    let archive_name = archive::archive_name(&version, WINDOWS_TARGET)?;
    let manifest_digest = listed_digest(listed, &version, "SHA256SUMS")?;
    let archive_digest = listed_digest(listed, &version, &archive_name)?;
    let download_base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
    let manifest = github
        .get(&format!("{download_base}/SHA256SUMS"), 64 * 1024)
        .await?;
    let archive = github
        .get(
            &format!("{download_base}/{archive_name}"),
            archive::MAX_ARCHIVE_BYTES,
        )
        .await?;
    verify_previous(
        &manifest,
        &archive,
        &archive_name,
        &manifest_digest,
        &archive_digest,
    )?;
    Ok(PreviousWindowsRelease {
        version,
        manifest,
        archive,
    })
}

/// Select the greatest published stable release other than the candidate.
/// This is GitHub's latest release, or the one before it when the candidate
/// itself is already published (a recovered Release run). Fail closed when a
/// stable release tag is not canonical `vX.Y.Z` or is newer than the candidate.
fn select_previous(
    releases: &[PublishedRelease],
    candidate: release::Version,
) -> Result<release::Version> {
    let mut previous = None;
    for listed in releases
        .iter()
        .filter(|listed| !listed.draft && !listed.prerelease)
    {
        let version = listed
            .tag_name
            .parse::<release::Version>()
            .ok()
            .filter(|version| listed.tag_name == format!("v{version}"))
            .with_context(|| {
                format!(
                    "published stable release tag {:?} is not vX.Y.Z",
                    listed.tag_name
                )
            })?;
        if version == candidate {
            continue;
        }
        ensure!(
            version < candidate,
            "published release v{version} is newer than candidate v{candidate}"
        );
        previous = previous.max(Some(version));
    }
    previous.with_context(|| format!("no published stable release precedes v{candidate}"))
}

fn listed_digest(listed: &PublishedRelease, version: &str, name: &str) -> Result<String> {
    let mut matches = listed.assets.iter().filter(|asset| asset.name == name);
    let asset = matches
        .next()
        .with_context(|| format!("previous release v{version} lacks {name}"))?;
    ensure!(
        matches.next().is_none(),
        "previous release v{version} lists {name} more than once"
    );
    ensure!(
        asset.browser_download_url
            == format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"),
        "previous release asset URL differs from its immutable version path"
    );
    let digest = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .with_context(|| {
            format!("previous release v{version} lacks a SHA-256 digest for {name}")
        })?;
    Ok(digest.to_owned())
}

fn verify_previous(
    manifest: &[u8],
    archive_bytes: &[u8],
    archive_name: &str,
    manifest_digest: &str,
    archive_digest: &str,
) -> Result<()> {
    ensure!(
        archive::digest(manifest) == manifest_digest,
        "previous release SHA256SUMS differs from its GitHub asset digest"
    );
    let expected = archive::expected_digest(manifest, archive_name)?;
    ensure!(
        archive::digest(archive_bytes) == expected,
        "previous release Windows archive differs from its SHA256SUMS"
    );
    ensure!(
        expected == archive_digest,
        "previous release SHA256SUMS and GitHub disagree on the Windows archive"
    );
    Ok(())
}

fn read_bounded(path: &Path, limit: usize, description: &str) -> Result<Vec<u8>> {
    let parent = Directory::open(
        path.parent().context("checked file has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let name = path.file_name().context("checked file has no filename")?;
    let mut file = parent.read(name)?;
    let before = regular_file_info(&file)?;
    ensure!(
        before.len <= limit as u64,
        "{description} exceeds size limit"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "{description} exceeds size limit");
    let after = regular_file_info(&file)?;
    parent.verify(name, &file)?;
    let named = regular_file_info(&parent.read(name)?)?;
    ensure!(
        before.identity == after.identity
            && before.identity == named.identity
            && before.len == bytes.len() as u64
            && before.len == after.len
            && before.len == named.len,
        "{description} changed while reading"
    );
    Ok(bytes)
}

fn within(path: &Path, root: &Path, description: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(path).with_context(|| format!("resolve {description}"))?;
    let root = fs::canonicalize(root).with_context(|| format!("resolve {description} root"))?;
    ensure!(
        path.starts_with(root),
        "{description} escaped its isolated root"
    );
    Ok(path)
}

fn environment_path(environment: &[(OsString, OsString)], name: &str) -> Result<PathBuf> {
    environment
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| PathBuf::from(value))
        .with_context(|| format!("missing isolated {name}"))
}

struct MiseInstall {
    root: PathBuf,
    project: PathBuf,
    executable: PathBuf,
    environment: Vec<(OsString, OsString)>,
    data: PathBuf,
    engine_cache: PathBuf,
    commands: Vec<CommandEvidence>,
    memory_attempted: bool,
}

impl MiseInstall {
    fn new(root: &Path, executable: &Path) -> Result<Self> {
        let project = root.join("project");
        fs::create_dir(&project)?;
        let mut environment = mise_isolation::prepare(root, &project)?;
        environment.extend([
            ("CI".into(), "1".into()),
            ("MISE_NO_HOOKS".into(), "1".into()),
            ("MISE_TASK_RUN_AUTO_INSTALL".into(), "false".into()),
        ]);
        fs::write(project.join("mise.toml"), MISE_CONFIG)?;
        let engine_cache = root.join("cold-engine-cache");
        let config = environment_path(&environment, "APPDATA")?.join("kuru/config.toml");
        fs::create_dir_all(config.parent().context("Kuru config has no parent")?)?;
        fs::write(
            config,
            toml::to_string(&KuruConfig {
                memory: MemoryConfig {
                    offline: true,
                    cache_dir: &engine_cache,
                },
            })?,
        )?;
        Ok(Self {
            root: root.to_owned(),
            project,
            executable: executable.to_owned(),
            environment,
            data: root.join("cold-kuru-data"),
            engine_cache,
            commands: Vec::new(),
            memory_attempted: false,
        })
    }

    fn command(&self) -> command::Command {
        let mut command = command::Command::new(&self.executable);
        // Kuru's selected memory owner must outlive each short mise command.
        // Only an explicit IndependentService request may leave this Job.
        #[cfg(windows)]
        command.fixture_allow_independent_service();
        command.env_clear().current_dir(&self.project);
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        command
    }

    async fn output(&self, phase: &'static str, arguments: &[OsString]) -> Result<Output> {
        let mut child = self.command();
        child.args(arguments);
        command::bounded_output(&mut child, COMMAND_DEADLINE, OUTPUT_LIMIT)
            .await
            .with_context(|| format!("native mise {phase} command did not settle"))
    }

    async fn success(&mut self, name: &'static str, arguments: &[&str]) -> Result<String> {
        let arguments = arguments.iter().map(OsString::from).collect::<Vec<_>>();
        let output = self.output(name, &arguments).await?;
        ensure!(
            output.status.success(),
            "{name} failed: {}",
            diagnostic(&output)
        );
        self.commands.push(CommandEvidence {
            name,
            status: "success",
        });
        String::from_utf8(output.stdout).context("mise stdout is not UTF-8")
    }

    async fn kuru(&mut self, name: &'static str, arguments: &[String]) -> Result<Value> {
        let mut command = vec![
            "exec".into(),
            "--".into(),
            "kuru".into(),
            "-C".into(),
            self.project.as_os_str().to_owned(),
            "--data-dir".into(),
            self.data.as_os_str().to_owned(),
            "--provider".into(),
            "demo".into(),
            "--mode".into(),
            "freudian".into(),
            "--no-dream".into(),
        ];
        command.extend(arguments.iter().map(OsString::from));
        self.memory_attempted = true;
        let output = self.output(name, &command).await?;
        ensure!(
            output.status.success(),
            "{name} failed: {}",
            diagnostic(&output)
        );
        self.commands.push(CommandEvidence {
            name,
            status: "success",
        });
        machine_json(&output.stdout)
    }

    async fn retire_memory(&mut self) -> Result<()> {
        if !self.memory_attempted {
            return Ok(());
        }
        ensure!(
            self.project == self.root.join("project")
                && self.data == self.root.join("cold-kuru-data"),
            "published verification cleanup changed its isolated project or data root"
        );
        let arguments = vec![
            OsString::from("exec"),
            OsString::from("--"),
            OsString::from("kuru"),
            OsString::from("-C"),
            self.project.as_os_str().to_owned(),
            OsString::from("--data-dir"),
            self.data.as_os_str().to_owned(),
            OsString::from("memory"),
            OsString::from("purge"),
            OsString::from("--yes"),
        ];
        let output = self.output("memory-cleanup", &arguments).await?;
        ensure!(
            output.status.success(),
            "isolated memory cleanup returned {}",
            output.status
        );
        Ok(())
    }

    async fn config_count(&mut self, name: &'static str) -> Result<usize> {
        let value: Value =
            serde_json::from_str(&self.success(name, &["config", "ls", "--json"]).await?)?;
        let entries = value
            .as_array()
            .context("mise config ls did not return an array")?;
        for entry in entries {
            let path = entry["path"]
                .as_str()
                .context("mise config entry lacks path")?;
            within(Path::new(path), &self.root, "mise configuration")?;
        }
        Ok(entries.len())
    }

    async fn verify_isolation(&mut self) -> Result<usize> {
        let count = self.config_count("mise-config-before").await?;
        for (setting, expected) in [
            ("use_versions_host", "false"),
            ("use_versions_host_track", "false"),
            ("netrc", "false"),
            ("github.gh_cli_tokens", "false"),
            ("github.use_git_credentials", "false"),
        ] {
            ensure!(
                self.success("mise-setting", &["settings", "get", setting])
                    .await?
                    .trim()
                    == expected,
                "unexpected effective mise setting {setting}"
            );
        }
        let replacements = self
            .success(
                "mise-url-replacements",
                &["settings", "ls", "url_replacements", "--json"],
            )
            .await?;
        validate_url_replacements(&replacements)?;
        Ok(count)
    }
}

fn validate_url_replacements(output: &str) -> Result<()> {
    let value: Value =
        serde_json::from_str(output).context("mise url_replacements did not return JSON")?;
    ensure!(
        value.is_object(),
        "mise url_replacements did not return a JSON object"
    );
    let replacements: MiseSettings = serde_json::from_value(value)
        .context("mise url_replacements did not return the expected JSON object")?;
    ensure!(
        replacements
            .url_replacements
            .iter()
            .all(|(source, destination)| {
                !source.contains("http://")
                    && !source.contains("https://")
                    && !destination.contains("http://")
                    && !destination.contains("https://")
            }),
        "published verification cannot use URL replacements"
    );
    Ok(())
}

fn diagnostic(output: &Output) -> String {
    const LIMIT: usize = 8 * 1024;
    format!(
        "status={}; stdout={}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(LIMIT)]),
        String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(LIMIT)])
    )
}

fn value_string<'a>(value: &'a Value, key: &str, description: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("{description} lacks {key}"))
}

fn machine_json(stdout: &[u8]) -> Result<Value> {
    serde_json::from_slice(stdout).context("installed Kuru stdout is not JSON")
}

fn checked_run_url(value: &str) -> Result<&str> {
    let url = Url::parse(value)?;
    let run = url
        .path()
        .strip_prefix("/replygirl/kuru/actions/runs/")
        .unwrap_or_default();
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && !run.is_empty()
            && run.bytes().all(|byte| byte.is_ascii_digit())
            && url.query().is_none()
            && url.fragment().is_none(),
        "release run URL is not the canonical repository Actions run"
    );
    Ok(value)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn finish_isolated_verification(
    temporary: tempfile::TempDir,
    verified: Result<Receipt>,
    cleanup: Result<()>,
) -> Result<Receipt> {
    let mut receipt = match (verified, cleanup) {
        (Ok(receipt), Ok(())) => receipt,
        (Err(error), Ok(())) => {
            let close = temporary.close();
            return match close {
                Ok(()) => Err(error),
                Err(close) => Err(error).context(format!(
                    "isolated verification also failed to remove its retired root: {close}"
                )),
            };
        }
        (Ok(_), Err(cleanup)) => {
            let retained = temporary.keep();
            return Err(cleanup).with_context(|| {
                format!("isolated memory cleanup failed; root retained at {retained:?}")
            });
        }
        (Err(error), Err(cleanup)) => {
            let retained = temporary.keep();
            return Err(error).context(format!(
                "isolated memory cleanup also failed: {cleanup:#}; root retained at {retained:?}"
            ));
        }
    };
    temporary
        .close()
        .context("isolated published verification cleanup was not confirmed")?;
    receipt.cleanup_confirmed = true;
    Ok(receipt)
}

pub async fn run(options: Options) -> Result<()> {
    ensure!(
        cfg!(windows),
        "verify-published-windows requires native Windows"
    );
    let version = options.version.parse::<release::Version>()?.to_string();
    let expected_sha = release::checked_sha(&options.expected_sha)?.to_owned();
    let run_url = checked_run_url(&options.run_url)?.to_owned();
    let mise = absolute(&options.mise)?;
    ensure!(
        mise.is_file(),
        "--mise must name the resolved native mise executable"
    );
    let manifest_path = absolute(&options.manifest)?;
    let evidence_path = absolute(&options.evidence)?;

    let github = PublicGitHub::new()?;
    let published = github.release(&version).await?;
    validate_release(&published, &version)?;
    ensure!(
        github.tag_commit(&version).await? == expected_sha,
        "published tag resolves to a different commit"
    );
    let release_url = format!("https://github.com/{REPOSITORY}/releases/tag/v{version}");
    let download_base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
    let checksum_bytes = github
        .get(&format!("{download_base}/SHA256SUMS"), 64 * 1024)
        .await?;
    let checksums = parse_checksums(&checksum_bytes, &version)?;
    let checksum_manifest_sha256 = archive::digest(&checksum_bytes);
    validate_asset_digests(&published, &checksums, &checksum_manifest_sha256)?;
    let archive_name = archive::archive_name(&version, WINDOWS_TARGET)?;
    let archive_bytes = github
        .get(
            &format!("{download_base}/{archive_name}"),
            archive::MAX_ARCHIVE_BYTES,
        )
        .await?;
    let archive_sha256 = archive::digest(&archive_bytes);
    ensure!(
        checksums.get(&archive_name) == Some(&archive_sha256),
        "published Windows archive differs from SHA256SUMS"
    );
    let (executable, marked) = archive::extract_core(
        &archive_bytes,
        targets::find(WINDOWS_TARGET)?,
        archive::MAX_ARCHIVE_BYTES,
    )?;
    ensure!(
        marked,
        "published core is missing its required shell support marker"
    );
    let support_name = crate::shell_support::archive_name(&version, WINDOWS_TARGET)?;
    let support_bytes = github
        .get(
            &format!("{download_base}/{support_name}"),
            crate::shell_support::MAX_ENVELOPE_BYTES,
        )
        .await?;
    ensure!(
        checksums.get(&support_name) == Some(&archive::digest(&support_bytes)),
        "published Windows shell support differs from SHA256SUMS"
    );
    let support = crate::shell_support::decode(&support_bytes, targets::find(WINDOWS_TARGET)?)?;
    let executable_sha256 = archive::digest(&executable);

    let manifest: EngineManifest = serde_json::from_slice(&read_bounded(
        &manifest_path,
        METADATA_LIMIT,
        "Dolt asset manifest",
    )?)?;
    ensure!(
        manifest.schema_version == 1,
        "unsupported Dolt asset manifest schema"
    );
    let engine_assets = manifest
        .assets
        .iter()
        .filter(|asset| asset.target == WINDOWS_TARGET)
        .collect::<Vec<_>>();
    ensure!(
        engine_assets.len() == 1,
        "Dolt manifest must contain exactly one Windows asset"
    );
    let engine_asset = engine_assets[0];

    let temporary = tempfile::tempdir()?;
    let root = temporary.path().to_owned();
    ensure!(
        fs::read_dir(&root)?.next().is_none(),
        "isolated verification root was not empty"
    );
    ensure!(
        !evidence_path.starts_with(&root),
        "evidence path must be outside the isolated verification root"
    );
    let mut install = MiseInstall::new(&root, &mise)?;
    let verified: Result<Receipt> = async {
        let mise_version = install.success("mise-version", &["--version"]).await?;
        ensure!(
            mise_version.trim().starts_with("2026.9.4") && mise_version.trim().len() <= 256,
            "workflow mise version differs from 2026.9.4"
        );
        let isolated_config_count = install.verify_isolation().await?;
        let selector = format!("github:{REPOSITORY}@{version}");
        install.success("mise-use", &["use", &selector]).await?;
        install.config_count("mise-config-after").await?;
        let selected = install.success("mise-which", &["which", "kuru"]).await?;
        let selected = within(
            Path::new(selected.trim()),
            &environment_path(&install.environment, "MISE_DATA_DIR")?.join("installs"),
            "mise-installed executable",
        )?;
        let installed = read_bounded(
            &selected,
            archive::MAX_ARCHIVE_BYTES,
            "mise-installed executable",
        )?;
        let installed_sha256 = archive::digest(&installed);
        ensure!(
            installed_sha256 == executable_sha256,
            "mise-installed executable differs from the independently verified archive"
        );
        let reported = install
            .success("installed-version", &["exec", "--", "kuru", "--version"])
            .await?;
        ensure!(
            reported.trim() == format!("kuru {version}"),
            "installed Kuru reports a different version"
        );
        for (name, shell) in [
            ("completions/kuru.bash", "bash"),
            ("completions/_kuru", "zsh"),
            ("completions/kuru.fish", "fish"),
            ("completions/kuru.ps1", "powershell"),
        ] {
            let output = install
                .success(
                    "installed-shell-support",
                    &["exec", "--", "kuru", "completions", shell],
                )
                .await?;
            ensure!(
                support.get(name) == Some(output.as_bytes()),
                "published shell support differs from the installed CLI"
            );
        }
        let man = install
            .success("installed-man", &["exec", "--", "kuru", "man"])
            .await?;
        ensure!(
            support.get("man/kuru.1") == Some(man.as_bytes()),
            "published man page differs from the installed CLI"
        );
        ensure!(
            !install.data.exists() && !install.engine_cache.exists(),
            "published runtime acceptance did not begin with cold state"
        );

        let first = install
            .kuru(
                "demo-conversation",
                &[
                    "run".into(),
                    "Published Windows verification".into(),
                    "--json".into(),
                ],
            )
            .await?;
        ensure!(
            !value_string(&first, "text", "first conversation")?.is_empty(),
            "published Kuru returned an empty first response"
        );
        let session = value_string(&first, "session", "first conversation")?.to_owned();
        let before = install
            .kuru("memory-status-before", &["memory".into(), "status".into()])
            .await?;
        ensure!(
            before["engine"] == "dolt",
            "published runtime did not use Dolt"
        );
        let first_revision = value_string(&before, "revision", "first memory status")?.to_owned();
        let second = install
            .kuru(
                "resumed-conversation",
                &[
                    "--resume".into(),
                    session.clone(),
                    "run".into(),
                    "Resume the exact published verification session".into(),
                    "--json".into(),
                ],
            )
            .await?;
        ensure!(
            !value_string(&second, "text", "resumed conversation")?.is_empty(),
            "published Kuru returned an empty resumed response"
        );
        ensure!(
            second["session"] == session,
            "published Kuru did not resume the same session"
        );
        let after = install
            .kuru("memory-status-after", &["memory".into(), "status".into()])
            .await?;
        let second_revision = value_string(&after, "revision", "second memory status")?.to_owned();
        ensure!(
            first_revision != second_revision,
            "published conversations did not create distinct revisions"
        );
        let history = install
            .kuru(
                "memory-history",
                &[
                    "memory".into(),
                    "history".into(),
                    "--limit".into(),
                    "100".into(),
                ],
            )
            .await?;
        for revision in [&first_revision, &second_revision] {
            ensure!(
                history
                    .as_array()
                    .context("memory history is not an array")?
                    .iter()
                    .any(|entry| entry["hash"] == *revision),
                "published conversation revision is absent from memory history"
            );
        }
        let sessions = install.kuru("sessions", &["sessions".into()]).await?;
        ensure!(
            sessions
                .as_array()
                .context("sessions is not an array")?
                .iter()
                .any(|entry| entry["id"] == session && entry["turns"] == 2),
            "published session listing does not contain two durable turns"
        );

        let engine_root = install
            .engine_cache
            .join(&manifest.version)
            .join(WINDOWS_TARGET);
        let engine_bytes = read_bounded(
            &engine_root.join("dolt.exe"),
            archive::MAX_ARCHIVE_BYTES,
            "extracted Dolt executable",
        )?;
        let license_bytes = read_bounded(
            &engine_root.join("LICENSES"),
            archive::MAX_ARCHIVE_BYTES,
            "extracted Dolt licenses",
        )?;
        ensure!(
            engine_bytes.len() as u64 == engine_asset.executable_bytes
                && archive::digest(&engine_bytes) == engine_asset.executable_sha256,
            "extracted Dolt executable differs from the checked-out manifest"
        );
        ensure!(
            license_bytes.len() as u64 == engine_asset.license_bytes
                && archive::digest(&license_bytes) == engine_asset.license_sha256,
            "extracted Dolt licenses differ from the checked-out manifest"
        );

        let commands = std::mem::take(&mut install.commands);
        let receipt = Receipt {
            schema_version: 1,
            runner_os: std::env::consts::OS,
            runner_arch: std::env::consts::ARCH,
            repository: REPOSITORY,
            release_url,
            run_url,
            version,
            commit: expected_sha,
            checksum_manifest_sha256,
            archive_sha256,
            executable_sha256,
            installed_sha256,
            mise_version: mise_version.trim().to_owned(),
            isolated_config_count,
            commands,
            session,
            first_revision,
            second_revision,
            engine: EngineEvidence {
                version: manifest.version,
                executable_sha256: engine_asset.executable_sha256.clone(),
                license_sha256: engine_asset.license_sha256.clone(),
            },
            cleanup_confirmed: false,
        };
        Ok(receipt)
    }
    .await;

    // The installed CLI's purge takes the project's authenticated maintenance
    // permit, retires its idle owner and removes only this disposable root's
    // memory. It must settle before TempDir may remove the enclosing files.
    let cleanup = install.retire_memory().await;
    drop(install);
    let receipt = finish_isolated_verification(temporary, verified, cleanup)?;
    let bytes = serde_json::to_vec_pretty(&receipt)?;
    ensure!(
        bytes.len() <= RECEIPT_LIMIT,
        "published verification receipt exceeds size limit"
    );
    let mut evidence = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&evidence_path)
        .context("create published verification receipt")?;
    evidence.write_all(&bytes)?;
    evidence.sync_all()?;
    println!("{}", String::from_utf8(bytes)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn published(
        version: &str,
        hashes: &BTreeMap<String, String>,
        manifest_hash: &str,
    ) -> PublishedRelease {
        let mut assets = expected_assets(version)
            .unwrap()
            .into_iter()
            .map(|name| ReleaseAsset {
                digest: Some(format!(
                    "sha256:{}",
                    if name == "SHA256SUMS" {
                        manifest_hash
                    } else {
                        &hashes[&name]
                    }
                )),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"
                ),
                name,
            })
            .collect::<Vec<_>>();
        assets.sort_by(|left, right| left.name.cmp(&right.name));
        PublishedRelease {
            tag_name: format!("v{version}"),
            draft: false,
            prerelease: false,
            assets,
        }
    }

    fn sums(version: &str) -> (Vec<u8>, BTreeMap<String, String>) {
        let names = expected_assets(version).unwrap();
        let text = names
            .into_iter()
            .filter(|name| name != "SHA256SUMS")
            .enumerate()
            .map(|(index, name)| format!("{index:064x}  {name}\n"))
            .collect::<String>();
        let bytes = text.into_bytes();
        let parsed = parse_checksums(&bytes, version).unwrap();
        (bytes, parsed)
    }

    #[test]
    fn publication_inventory_and_digests_are_exact() {
        let (bytes, hashes) = sums("1.2.3");
        let mut release = published("1.2.3", &hashes, &archive::digest(&bytes));
        validate_release(&release, "1.2.3").unwrap();
        validate_asset_digests(&release, &hashes, &archive::digest(&bytes)).unwrap();

        release.assets.push(ReleaseAsset {
            name: release.assets[0].name.clone(),
            browser_download_url: release.assets[0].browser_download_url.clone(),
            digest: release.assets[0].digest.clone(),
        });
        assert!(validate_release(&release, "1.2.3").is_err());
        release.assets.pop();
        release.assets[0].digest = Some(format!("sha256:{}", "f".repeat(64)));
        assert!(validate_asset_digests(&release, &hashes, &archive::digest(&bytes)).is_err());
        release.assets[0].browser_download_url = "https://example.invalid/archive".into();
        assert!(validate_release(&release, "1.2.3").is_err());
    }

    #[test]
    fn checksum_inventory_rejects_missing_duplicate_and_extra_entries() {
        let (bytes, _) = sums("1.2.3");
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            parse_checksums(
                text.lines()
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes(),
                "1.2.3"
            )
            .is_err()
        );
        assert!(parse_checksums(format!("{text}{text}").as_bytes(), "1.2.3").is_err());
        assert!(
            parse_checksums(
                format!("{text}{}  extra.zip\n", "a".repeat(64)).as_bytes(),
                "1.2.3"
            )
            .is_err()
        );
    }

    fn listed(tag: &str, draft: bool, prerelease: bool) -> PublishedRelease {
        PublishedRelease {
            tag_name: tag.into(),
            draft,
            prerelease,
            assets: Vec::new(),
        }
    }

    fn version(value: &str) -> release::Version {
        value.parse().unwrap()
    }

    #[test]
    fn previous_release_is_the_greatest_older_stable_release() {
        let releases = [
            listed("v0.10.0", true, false),
            listed("v0.9.1-rc.1", false, true),
            listed("v0.8.0", false, false),
            listed("v0.9.0", false, false),
            listed("v0.4.2", false, false),
        ];
        assert_eq!(
            select_previous(&releases, version("0.10.0")).unwrap(),
            version("0.9.0")
        );

        // A recovered run whose candidate is already public uses the one before.
        let recovered = [
            listed("v0.10.0", false, false),
            listed("v0.9.0", false, false),
        ];
        assert_eq!(
            select_previous(&recovered, version("0.10.0")).unwrap(),
            version("0.9.0")
        );

        for (releases, candidate) in [
            (vec![listed("v0.10.0", false, false)], "0.10.0"),
            (vec![], "0.10.0"),
            (vec![listed("v0.11.0", false, false)], "0.10.0"),
            (vec![listed("0.9.0", false, false)], "0.10.0"),
            (vec![listed("v0.09.0", false, false)], "0.10.0"),
            (vec![listed("vv0.9.0", false, false)], "0.10.0"),
            (vec![listed("nightly", false, false)], "0.10.0"),
        ] {
            assert!(
                select_previous(&releases, version(candidate)).is_err(),
                "accepted {:?}",
                releases.iter().map(|r| &r.tag_name).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn previous_release_assets_are_authenticated_before_use() {
        let name = archive::archive_name("0.9.0", WINDOWS_TARGET).unwrap();
        let archive_bytes = b"windows zip".to_vec();
        let archive_digest = archive::digest(&archive_bytes);
        let manifest = format!("{archive_digest}  {name}\n").into_bytes();
        let manifest_digest = archive::digest(&manifest);
        let asset = |name: &str, digest: Option<String>| ReleaseAsset {
            name: name.into(),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/v0.9.0/{name}"
            ),
            digest,
        };
        let mut release = listed("v0.9.0", false, false);
        release.assets = vec![
            asset("SHA256SUMS", Some(format!("sha256:{manifest_digest}"))),
            asset(&name, Some(format!("sha256:{archive_digest}"))),
        ];
        assert_eq!(
            listed_digest(&release, "0.9.0", "SHA256SUMS").unwrap(),
            manifest_digest
        );
        assert_eq!(
            listed_digest(&release, "0.9.0", &name).unwrap(),
            archive_digest
        );
        verify_previous(
            &manifest,
            &archive_bytes,
            &name,
            &manifest_digest,
            &archive_digest,
        )
        .unwrap();

        let other = "f".repeat(64);
        assert!(
            verify_previous(
                &manifest,
                b"altered",
                &name,
                &manifest_digest,
                &archive_digest
            )
            .is_err()
        );
        assert!(
            verify_previous(&manifest, &archive_bytes, &name, &other, &archive_digest).is_err()
        );
        assert!(
            verify_previous(&manifest, &archive_bytes, &name, &manifest_digest, &other).is_err()
        );
        let foreign = format!("{archive_digest}  kuru-0.9.0-other.zip\n").into_bytes();
        assert!(
            verify_previous(
                &foreign,
                &archive_bytes,
                &name,
                &archive::digest(&foreign),
                &archive_digest
            )
            .is_err()
        );

        release.assets[1].digest = None;
        assert!(listed_digest(&release, "0.9.0", &name).is_err());
        release.assets[1].digest = Some(format!("sha1:{archive_digest}"));
        assert!(listed_digest(&release, "0.9.0", &name).is_err());
        release.assets[1].digest = Some(format!("sha256:{archive_digest}"));
        release.assets[1].browser_download_url = "https://example.invalid/kuru.zip".into();
        assert!(listed_digest(&release, "0.9.0", &name).is_err());
        release.assets[1] = asset(&name, Some(format!("sha256:{archive_digest}")));
        release
            .assets
            .push(asset(&name, Some(format!("sha256:{archive_digest}"))));
        assert!(listed_digest(&release, "0.9.0", &name).is_err());
        release.assets.truncate(1);
        assert!(listed_digest(&release, "0.9.0", &name).is_err());
    }

    #[test]
    fn public_selector_and_receipt_do_not_contain_endpoint_or_raw_output_fields() {
        let selector = format!(
            "github:{REPOSITORY}@{}",
            "1.2.3".parse::<release::Version>().unwrap()
        );
        assert_eq!(selector, "github:replygirl/kuru@1.2.3");
        assert!(!selector.contains("http"));
        assert!(checked_run_url("https://github.com/replygirl/kuru/actions/runs/42").is_ok());
        assert!(checked_run_url("https://example.invalid/run").is_err());
        let receipt = Receipt {
            schema_version: 1,
            runner_os: "windows",
            runner_arch: "x86_64",
            repository: REPOSITORY,
            release_url: "https://github.com/replygirl/kuru/releases/tag/v1.2.3".into(),
            run_url: "https://github.com/replygirl/kuru/actions/runs/42".into(),
            version: "1.2.3".into(),
            commit: "a".repeat(40),
            checksum_manifest_sha256: "b".repeat(64),
            archive_sha256: "b".repeat(64),
            executable_sha256: "c".repeat(64),
            installed_sha256: "c".repeat(64),
            mise_version: "2026.9.4".into(),
            isolated_config_count: 1,
            commands: vec![
                CommandEvidence {
                    name: "mise-use",
                    status: "success",
                },
                CommandEvidence {
                    name: "demo-conversation",
                    status: "success",
                },
            ],
            session: "session".into(),
            first_revision: "d".repeat(64),
            second_revision: "e".repeat(64),
            engine: EngineEvidence {
                version: "2.3.3".into(),
                executable_sha256: "f".repeat(64),
                license_sha256: "0".repeat(64),
            },
            cleanup_confirmed: true,
        };
        let text = serde_json::to_string(&receipt).unwrap();
        assert!(text.len() < RECEIPT_LIMIT);
        for forbidden in ["stdout", "stderr", "token", "oauth", "proxy"] {
            assert!(!text.contains(forbidden));
        }
    }

    #[test]
    fn held_file_reads_are_bounded_and_complete() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input");
        fs::write(&path, b"1234").unwrap();
        assert_eq!(read_bounded(&path, 4, "input").unwrap(), b"1234");
        assert!(read_bounded(&path, 3, "input").is_err());
    }

    #[test]
    fn runtime_machine_fields_are_read_from_stdout_with_informational_stderr() {
        let stderr = b"Kuru stores local conversation memory on this device.\n";
        let value = machine_json(br#"{"session":"session-1","text":"answer"}"#).unwrap();
        assert_eq!(
            value_string(&value, "session", "conversation").unwrap(),
            "session-1"
        );
        assert_eq!(
            value_string(&value, "text", "conversation").unwrap(),
            "answer"
        );
        assert!(!stderr.is_empty());
        assert!(machine_json(b"notice before json").is_err());
        assert!(value_string(&json!({"session": "session-1"}), "text", "conversation").is_err());
    }

    #[test]
    fn failed_verification_retains_uncertain_root_and_primary_phase() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_owned();
        let primary: Result<Receipt> = Err(anyhow::anyhow!("demo-conversation phase failed"));
        let cleanup = Err(anyhow::anyhow!("authenticated retirement refused"));
        let error = finish_isolated_verification(temporary, primary, cleanup).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("demo-conversation phase failed"),
            "{message}"
        );
        assert!(
            message.contains("authenticated retirement refused"),
            "{message}"
        );
        assert!(message.contains("root retained"), "{message}");
        assert!(root.is_dir(), "uncertain root was deleted");
        fs::remove_dir_all(root).unwrap();

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().to_owned();
        let primary: Result<Receipt> = Err(anyhow::anyhow!("memory-status phase failed"));
        let error = finish_isolated_verification(temporary, primary, Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("memory-status phase failed"));
        assert!(
            !root.exists(),
            "retired fixture root remained after failure"
        );
    }

    #[test]
    fn optional_url_replacements_require_a_json_object_of_safe_strings() {
        validate_url_replacements("{}").unwrap();
        validate_url_replacements(r#"{"url_replacements":{"regex:^github$":"mirror"}}"#).unwrap();

        for invalid in [
            "{",
            "[]",
            r#"{"url_replacements":{"regex:^github$":false}}"#,
            r#"{"url_replacements":{"https://github.com":"mirror"}}"#,
            r#"{"url_replacements":{"mirror":"http://example.invalid"}}"#,
            r#"{"unexpected":{}}"#,
        ] {
            assert!(validate_url_replacements(invalid).is_err(), "{invalid}");
        }
    }

    #[cfg(windows)]
    fn native_mise() -> PathBuf {
        use kuru_platform::windows::process::configured_command;
        use std::ffi::OsStr;

        let directory = std::env::current_dir().unwrap();
        let environment = ["PATH", "PATHEXT", "SystemRoot"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
            .collect();
        configured_command(OsStr::new("mise"), &[], &directory, environment)
            .unwrap()
            .executable
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn isolated_native_mise_preflight_accepts_unset_url_replacements() {
        let temporary = tempfile::tempdir().unwrap();
        let mut install = MiseInstall::new(temporary.path(), &native_mise()).unwrap();

        assert!(install.verify_isolation().await.unwrap() > 0);
        assert!(
            install
                .commands
                .iter()
                .any(|command| command.name == "mise-url-replacements")
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn optional_url_replacements_command_failure_is_fatal() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = kuru_platform::windows::process::system_directory()
            .unwrap()
            .join("where.exe");
        let mut install = MiseInstall::new(temporary.path(), &executable).unwrap();

        let error = install
            .success(
                "mise-url-replacements",
                &["settings", "ls", "url_replacements", "--json"],
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("mise-url-replacements failed"), "{error}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn published_command_names_failed_phase_and_refuses_substituted_cleanup_root() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().join("missing-mise.exe");
        let mut install = MiseInstall::new(temporary.path(), &missing).unwrap();
        let error = install
            .output("demo-conversation", &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("native mise demo-conversation command did not settle"),
            "{error}"
        );

        let foreign = tempfile::tempdir().unwrap();
        let marker = foreign.path().join("untouched");
        fs::write(&marker, b"foreign").unwrap();
        install.memory_attempted = true;
        install.data = foreign.path().to_owned();
        let error = install.retire_memory().await.unwrap_err().to_string();
        assert!(error.contains("changed its isolated project or data root"));
        assert_eq!(fs::read(marker).unwrap(), b"foreign");
    }

    #[cfg(windows)]
    #[test]
    fn independent_service_child() {
        let Some(lock) = std::env::var_os("KURU_PUBLISHED_TEST_LOCK") else {
            return;
        };
        let release = PathBuf::from(std::env::var_os("KURU_PUBLISHED_TEST_RELEASE").unwrap());
        let held = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock)
            .unwrap();
        held.try_lock().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !release.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            release.exists(),
            "independent fixture release was not signaled"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn independent_service_starter() {
        use kuru_platform::windows::process::{Lifetime, NativeSpawnSpec};

        let Some(lock) = std::env::var_os("KURU_PUBLISHED_TEST_LOCK") else {
            return;
        };
        let release = std::env::var_os("KURU_PUBLISHED_TEST_RELEASE").unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut child = NativeSpawnSpec::new(executable, std::env::current_dir().unwrap());
        child.args = vec![
            "--exact".into(),
            "published_windows::tests::independent_service_child".into(),
            "--nocapture".into(),
        ];
        child.lifetime = Lifetime::IndependentService;
        child.environment = [
            ("KURU_PUBLISHED_TEST_LOCK".into(), lock),
            ("KURU_PUBLISHED_TEST_RELEASE".into(), release),
        ]
        .into();
        for name in ["SystemRoot", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(name) {
                child.environment.push((name.into(), value));
            }
        }
        let service = child.spawn().await.unwrap();
        let held = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(std::env::var_os("KURU_PUBLISHED_TEST_LOCK").unwrap())
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match held.try_lock() {
                Err(std::fs::TryLockError::WouldBlock) => break,
                Ok(()) => held.unlock().unwrap(),
                Err(error) => panic!("service lock check failed: {error}"),
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "service did not retain its lock"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(service);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn published_command_allows_only_explicit_service_breakaway() {
        struct ReleaseOnDrop(PathBuf);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                let _ = fs::write(&self.0, b"release");
            }
        }

        let temporary = tempfile::tempdir().unwrap();
        let lock = temporary.path().join("service.lock");
        let release = temporary.path().join("release");
        let release_guard = ReleaseOnDrop(release.clone());
        let mut install =
            MiseInstall::new(temporary.path(), &std::env::current_exe().unwrap()).unwrap();
        install.environment.extend([
            (
                "KURU_PUBLISHED_TEST_LOCK".into(),
                lock.as_os_str().to_owned(),
            ),
            (
                "KURU_PUBLISHED_TEST_RELEASE".into(),
                release.as_os_str().to_owned(),
            ),
        ]);
        let output = install
            .output(
                "independent-service-regression",
                &[
                    "--exact".into(),
                    "published_windows::tests::independent_service_starter".into(),
                    "--nocapture".into(),
                ],
            )
            .await
            .unwrap();
        assert!(output.status.success(), "starter did not complete");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .unwrap();
        assert!(matches!(
            held.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(release_guard);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match held.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => (),
                Err(error) => panic!("service retirement lock check failed: {error}"),
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "service did not retire"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
