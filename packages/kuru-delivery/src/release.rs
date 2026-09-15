//! Conventional versioning and immutable GitHub release publication.
use crate::archive::digest;
use crate::command::Command;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub use crate::targets::TARGETS;
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub [u64; 3]);
impl std::str::FromStr for Version {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> Result<Self> {
        let input = input.strip_prefix('v').unwrap_or(input);
        let pieces: Vec<_> = input.split('.').collect();
        ensure!(pieces.len() == 3, "expected a stable X.Y.Z version");
        let mut result = [0; 3];
        for (index, part) in pieces.into_iter().enumerate() {
            ensure!(
                !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0')),
                "expected a stable X.Y.Z version"
            );
            result[index] = part.parse().context("version component is too large")?;
        }
        Ok(Self(result))
    }
}
impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0[0], self.0[1], self.0[2])
    }
}
pub fn checked_sha(value: &str) -> Result<&str> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "expected a full lowercase commit SHA"
    );
    Ok(value)
}
/// Select `root` independently of Git's inherited hook environment. Apply any
/// deliberate repository overrides, such as a private index, after this call.
pub fn rooted_command(root: &Path, program: &str) -> Command {
    crate::command::rooted(root, program)
}
pub async fn run(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    command_output(rooted_command(root, program).args(args)).await
}
async fn command_output(command: &mut Command) -> Result<String> {
    let program = crate::command::program(command)
        .to_string_lossy()
        .into_owned();
    let output = crate::command::output(
        command.env("GIT_TERMINAL_PROMPT", "0").kill_on_drop(true),
        Duration::from_secs(180),
    )
    .await
    .context("tool execution failed")?;
    ensure!(
        output.status.success(),
        "{program} failed ({})",
        output.status
    );
    ensure!(
        output.stdout.len() <= 4 * 1024 * 1024,
        "tool output exceeds limit"
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
pub async fn git(root: &Path, args: &[&str]) -> Result<String> {
    run(root, "git", args).await
}
pub async fn workspace_version(root: &Path, commit: Option<&str>) -> Result<Version> {
    let text = if let Some(commit) = commit {
        git(
            root,
            &["show", &format!("{}:Cargo.toml", checked_sha(commit)?)],
        )
        .await?
    } else {
        fs::read_to_string(root.join("Cargo.toml"))?
    };
    let data: toml::Value = toml::from_str(&text)?;
    data["workspace"]["package"]["version"]
        .as_str()
        .context("missing workspace version")?
        .parse()
}
pub async fn compute_version(root: &Path, bump: &str) -> Result<Version> {
    ensure!(
        ["auto", "major", "minor", "patch"].contains(&bump),
        "unsupported version bump"
    );
    if bump == "auto" {
        for tag in git(root, &["tag", "--points-at", "HEAD", "--list", "v*"])
            .await?
            .lines()
        {
            ensure!(
                tag.parse::<Version>().is_err(),
                "no commits since existing release tag"
            );
        }
    }
    let selected: Version = run(root, "cog", &["bump", &format!("--{bump}"), "--dry-run"])
        .await?
        .parse()?;
    for tag in git(root, &["tag", "--merged", "HEAD", "--list", "v*"])
        .await?
        .lines()
    {
        if let Ok(previous) = tag.parse::<Version>() {
            ensure!(selected > previous, "no new release version selected");
        }
    }
    Ok(selected)
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub version: String,
    pub tag: String,
    pub base_sha: String,
}
pub async fn plan(root: &Path, bump: &str) -> Result<Plan> {
    ensure!(
        ["auto", "major", "minor", "patch"].contains(&bump),
        "unsupported version bump"
    );
    ensure!(
        git(root, &["status", "--porcelain"]).await?.is_empty(),
        "release planning requires a clean checkout"
    );
    let head = git(root, &["rev-parse", "HEAD"]).await?;
    checked_sha(&head)?;
    let current = workspace_version(root, None).await?;
    let tags = git(root, &["tag", "--points-at", "HEAD", "--list", "v*"]).await?;
    let tagged = tags.lines().any(|tag| {
        tag.parse::<Version>()
            .is_ok_and(|version| version == current)
    });
    let prepared =
        git(root, &["log", "-1", "--format=%s"]).await? == format!("chore(release): v{current}");
    let selected = if tagged || prepared {
        current
    } else {
        compute_version(root, bump).await?
    };
    ensure!(
        selected >= current,
        "calculated version would downgrade workspace; choose auto or minor"
    );
    Ok(Plan {
        version: selected.to_string(),
        tag: format!("v{selected}"),
        base_sha: head,
    })
}
fn rewrite_version(text: &str, section: Option<&str>, selected: Version) -> Result<String> {
    let mut active = section.is_none();
    let mut changed = 0;
    let mut result = String::new();
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            active = section.is_none() || Some(trimmed) == section;
        }
        if active
            && line
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "version")
        {
            let start = line
                .find(['\"', '\''])
                .context("version must be a quoted string")?;
            let quote = line.as_bytes()[start] as char;
            let end = start
                + 1
                + line[start + 1..]
                    .find(quote)
                    .context("unterminated version")?;
            result.push_str(&line[..start + 1]);
            result.push_str(&selected.to_string());
            result.push_str(&line[end..]);
            changed += 1;
        } else {
            result.push_str(line);
        }
    }
    ensure!(changed == 1, "expected one version declaration");
    Ok(result)
}
pub fn stamp(root: &Path, selected: Version) -> Result<Vec<PathBuf>> {
    let manifest_path = root.join("Cargo.toml");
    let lock_path = root.join("Cargo.lock");
    let manifest_text = fs::read_to_string(&manifest_path)?;
    let lock_text = fs::read_to_string(&lock_path)?;
    let manifest: toml::Value = toml::from_str(&manifest_text)?;
    let current = manifest["workspace"]["package"]["version"]
        .as_str()
        .context("missing workspace version")?;
    let mut names = BTreeSet::new();
    for member in manifest["workspace"]["members"]
        .as_array()
        .context("missing workspace members")?
    {
        let path = root
            .join(member.as_str().context("member must be a path")?)
            .join("Cargo.toml");
        let data: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
        ensure!(
            data["package"]["version"]
                .get("workspace")
                .and_then(toml::Value::as_bool)
                == Some(true),
            "all release packages must inherit workspace version"
        );
        names.insert(
            data["package"]["name"]
                .as_str()
                .context("missing package name")?
                .to_owned(),
        );
    }
    let mut blocks = lock_text.split("[[package]]");
    let mut updated_lock = blocks.next().unwrap_or_default().to_owned();
    let mut seen = BTreeSet::new();
    for block in blocks {
        let data: toml::Value = toml::from_str(&format!("[[package]]{block}"))?;
        let package = &data["package"][0];
        let name = package["name"]
            .as_str()
            .context("missing lock package name")?;
        updated_lock.push_str("[[package]]");
        if names.contains(name) && package.get("source").is_none() {
            ensure!(
                seen.insert(name.to_owned()),
                "duplicate workspace package in lockfile"
            );
            ensure!(
                package["version"].as_str() == Some(current),
                "inconsistent workspace package versions"
            );
            updated_lock.push_str(&rewrite_version(block, None, selected)?);
        } else {
            updated_lock.push_str(block);
        }
    }
    ensure!(
        seen == names,
        "Cargo.lock must contain every workspace package"
    );
    let replacements = [
        (
            manifest_path,
            manifest_text.clone(),
            rewrite_version(&manifest_text, Some("[workspace.package]"), selected)?,
        ),
        (lock_path, lock_text, updated_lock),
    ];
    let mut staged = Vec::new();
    for (path, original, updated) in replacements {
        if original != updated {
            let mut temp = tempfile::NamedTempFile::new_in(root)?;
            temp.write_all(updated.as_bytes())?;
            temp.as_file()
                .set_permissions(fs::metadata(&path)?.permissions())?;
            temp.as_file().sync_all()?;
            staged.push((path, temp));
        }
    }
    let mut changed = Vec::new();
    for (path, temp) in staged {
        temp.persist(&path)?;
        changed.push(path.strip_prefix(root)?.to_path_buf());
    }
    Ok(changed)
}

#[derive(Clone)]
pub struct GitHub {
    pub repository: String,
    token: String,
    client: Client,
    api_base: Url,
    upload_base: Url,
}
impl GitHub {
    pub fn new(repository: &str, token: &str) -> Result<Self> {
        Self::with_endpoints(
            repository,
            token,
            "https://api.github.com/",
            "https://uploads.github.com/",
        )
    }
    /// Alternate endpoints permit local HTTP contract fixtures; production uses new().
    pub fn with_endpoints(repository: &str, token: &str, api: &str, uploads: &str) -> Result<Self> {
        let pieces: Vec<_> = repository.split('/').collect();
        ensure!(
            pieces.len() == 2
                && pieces.iter().all(|p| !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))),
            "expected owner/repository"
        );
        let api_base = Url::parse(api)?;
        let upload_base = Url::parse(uploads)?;
        for endpoint in [&api_base, &upload_base] {
            ensure!(
                endpoint.username().is_empty()
                    && endpoint.password().is_none()
                    && endpoint.query().is_none()
                    && endpoint.fragment().is_none()
                    && (endpoint.scheme() == "https"
                        || (endpoint.scheme() == "http"
                            && endpoint.host_str().is_some_and(|h| [
                                "127.0.0.1",
                                "localhost",
                                "[::1]"
                            ]
                            .contains(&h)))),
                "API endpoint must use HTTPS or local fixture HTTP"
            );
        }
        ensure!(!token.is_empty(), "GitHub token is required");
        Ok(Self {
            repository: repository.to_owned(),
            token: token.to_owned(),
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            api_base,
            upload_base,
        })
    }
    fn path(&self, suffix: &str) -> String {
        format!("repos/{}/{suffix}", self.repository)
    }
    async fn decode(response: reqwest::Response, missing: bool) -> Result<Option<Value>> {
        let status = response.status();
        if missing && status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            status.is_success(),
            "GitHub API request failed ({status}); inspect permissions and expected main head"
        );
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                body.len() + chunk.len() <= 4 * 1024 * 1024,
                "GitHub response exceeds limit"
            );
            body.extend_from_slice(&chunk);
        }
        let data: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body)?
        };
        ensure!(
            data.get("errors").is_none(),
            "GitHub rejected request; check permissions and expected main head"
        );
        Ok(Some(data))
    }
    pub async fn api(
        &self,
        method: Method,
        path: &str,
        payload: Option<Value>,
        missing: bool,
    ) -> Result<Option<Value>> {
        let mut request = self
            .client
            .request(method, self.api_base.join(path)?)
            .bearer_auth(&self.token)
            .header("User-Agent", "kuru-release")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(payload) = payload {
            request = request.json(&payload);
        }
        Self::decode(request.send().await?, missing).await
    }
    async fn required(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        self.api(method, path, body, false)
            .await?
            .context("missing GitHub response")
    }
    async fn upload(&self, id: u64, path: &Path) -> Result<()> {
        ensure!(
            fs::metadata(path)?.len() <= MAX_ARCHIVE,
            "release asset exceeds upload limit"
        );
        let mut url = self
            .upload_base
            .join(&self.path(&format!("releases/{id}/assets")))?;
        url.query_pairs_mut().append_pair(
            "name",
            path.file_name()
                .and_then(|s| s.to_str())
                .context("invalid asset name")?,
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .header("User-Agent", "kuru-release")
            .header("Content-Type", "application/octet-stream")
            .body(fs::read(path)?)
            .send()
            .await?;
        Self::decode(response, false).await?;
        Ok(())
    }
    async fn checksum_manifest(&self, id: u64) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .get(
                self.api_base
                    .join(&self.path(&format!("releases/assets/{id}")))?,
            )
            .bearer_auth(&self.token)
            .header("User-Agent", "kuru-release")
            .header("Accept", "application/octet-stream")
            .send()
            .await?;
        // GitHub may redirect asset downloads to a signed storage URL. Never
        // forward the API token, and never follow a downgrade to plain HTTP.
        if response.status() == StatusCode::FOUND {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .context("missing asset redirect")?
                .to_str()?;
            let url = Url::parse(location)?;
            ensure!(
                url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
                "invalid asset redirect"
            );
            response = self.client.get(url).send().await?;
        }
        ensure!(response.status().is_success(), "checksum download failed");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 4096,
                "checksum manifest exceeds limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
pub async fn commit_version(
    root: &Path,
    api: &GitHub,
    expected: &str,
    selected: Version,
) -> Result<String> {
    checked_sha(expected)?;
    ensure!(
        git(root, &["rev-parse", "HEAD"]).await? == expected,
        "checkout does not match expected release base"
    );
    ensure!(
        workspace_version(root, None).await? == selected,
        "workspace not stamped to selected version"
    );
    let diff = git(root, &["diff", "HEAD", "--name-only"]).await?;
    let paths: Vec<_> = diff.lines().collect();
    let unexpected: Vec<_> = paths
        .iter()
        .copied()
        .filter(|path| !["Cargo.toml", "Cargo.lock"].contains(path))
        .collect();
    ensure!(
        unexpected.is_empty(),
        "release stamp changed unrelated files: {}",
        unexpected.join(", ")
    );
    let current = api
        .required(Method::GET, &api.path("commits/main"), None)
        .await?;
    let head = checked_sha(current["sha"].as_str().context("missing main SHA")?)?;
    if head != expected {
        // Compare immutable SHAs, not a moving branch. The first descendant must
        // be precisely our stamp, even if other commits have since reached main.
        let comparison = api
            .required(
                Method::GET,
                &api.path(&format!("compare/{expected}...{head}?per_page=1")),
                None,
            )
            .await?;
        ensure!(
            comparison["status"] == "ahead" && comparison["merge_base_commit"]["sha"] == expected,
            "main diverged from expected release base"
        );
        if paths.is_empty() {
            return Ok(expected.to_owned());
        }
        let first = &comparison["commits"][0];
        let tree = stamped_tree(root).await?;
        ensure!(
            first["parents"]
                .as_array()
                .is_some_and(|p| p.len() == 1 && p[0]["sha"] == expected)
                && first["commit"]["message"] == format!("chore(release): v{selected}")
                && first["commit"]["tree"]["sha"] == tree,
            "main moved without the expected release commit; expected main head no longer matches"
        );
        return Ok(checked_sha(
            first["sha"]
                .as_str()
                .context("missing existing version commit SHA")?,
        )?
        .to_owned());
    }
    if paths.is_empty() {
        return Ok(expected.to_owned());
    }
    let additions = paths
        .into_iter()
        .map(|path| Ok(json!({"path":path,"contents":STANDARD.encode(fs::read(root.join(path))?)})))
        .collect::<Result<Vec<_>>>()?;
    let result = api.required(Method::POST, "graphql", Some(json!({"query":"mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid } } }", "variables":{"input":{"branch":{"repositoryNameWithOwner":api.repository,"branchName":"main"},"message":{"headline":format!("chore(release): v{selected}")},"expectedHeadOid":expected,"fileChanges":{"additions":additions}}}}))).await?;
    Ok(checked_sha(
        result["data"]["createCommitOnBranch"]["commit"]["oid"]
            .as_str()
            .context("missing signed commit SHA")?,
    )?
    .to_owned())
}
async fn stamped_tree(root: &Path) -> Result<String> {
    let temp = tempfile::tempdir()?;
    let index = temp.path().join("index");
    let mut result = String::new();
    for args in [
        &["read-tree", "HEAD"][..],
        &["add", "--", "Cargo.toml", "Cargo.lock"],
        &["write-tree"],
    ] {
        result = command_output(
            rooted_command(root, "git")
                .args(args)
                .env("GIT_INDEX_FILE", &index),
        )
        .await?;
    }
    Ok(checked_sha(&result)?.to_owned())
}
pub async fn tag_commit(api: &GitHub, selected: Version) -> Result<Option<String>> {
    let Some(reference) = api
        .api(
            Method::GET,
            &api.path(&format!("git/ref/tags/v{selected}")),
            None,
            true,
        )
        .await?
    else {
        return Ok(None);
    };
    let mut object = reference["object"].clone();
    for _ in 0..8 {
        let id = checked_sha(object["sha"].as_str().context("missing tag object SHA")?)?;
        match object["type"].as_str() {
            Some("commit") => return Ok(Some(id.to_owned())),
            Some("tag") => {
                object = api
                    .required(Method::GET, &api.path(&format!("git/tags/{id}")), None)
                    .await?["object"]
                    .clone()
            }
            _ => break,
        }
    }
    bail!("release tag does not resolve to a commit")
}
pub async fn ensure_tag(api: &GitHub, selected: Version, candidate: &str) -> Result<()> {
    checked_sha(candidate)?;
    if let Some(existing) = tag_commit(api, selected).await? {
        ensure!(
            existing == candidate,
            "existing release tag points to a different commit"
        );
        return Ok(());
    }
    let object = api.required(Method::POST, &api.path("git/tags"), Some(json!({"tag":format!("v{selected}"),"message":format!("Kuru v{selected}"),"object":candidate,"type":"commit"}))).await?;
    let id = checked_sha(object["sha"].as_str().context("missing new tag SHA")?)?;
    api.required(
        Method::POST,
        &api.path("git/refs"),
        Some(json!({"ref":format!("refs/tags/v{selected}"),"sha":id})),
    )
    .await?;
    Ok(())
}
fn archive_assets(directory: &Path, selected: Version) -> Result<BTreeMap<String, String>> {
    let expected: BTreeSet<_> = TARGETS
        .iter()
        .map(|target| crate::archive::archive_name(&selected.to_string(), target))
        .collect::<Result<_>>()?;
    let sidecars = expected
        .iter()
        .map(|name| format!("{name}.sha256"))
        .collect::<BTreeSet<_>>();
    let allowed = expected
        .iter()
        .chain(sidecars.iter())
        .cloned()
        .chain(std::iter::once("SHA256SUMS".into()))
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        ensure!(
            allowed.contains(&name) && entry.file_type()?.is_file(),
            "release candidate contains an unexpected or nonregular entry"
        );
        actual.insert(name);
    }
    ensure!(
        expected.is_subset(&actual) && sidecars.is_subset(&actual),
        "release candidate must contain exactly five supported native archives and sidecars"
    );
    let mut checksums = BTreeMap::new();
    for name in expected {
        let path = directory.join(&name);
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_ARCHIVE,
            "archive must be a bounded regular file"
        );
        let digest = digest(&fs::read(&path)?);
        ensure!(
            fs::read_to_string(directory.join(format!("{name}.sha256")))?.trim_end_matches('\n')
                == format!("{digest}  {name}"),
            "checksum mismatch: {name}"
        );
        checksums.insert(name, digest);
    }
    Ok(checksums)
}

fn checksum_manifest(checksums: &BTreeMap<String, String>) -> String {
    checksums
        .iter()
        .map(|(name, digest)| format!("{digest}  {name}\n"))
        .collect()
}

fn checked_notes(notes: &Path) -> Result<()> {
    let body = fs::read_to_string(notes)?;
    ensure!(
        !body.trim().is_empty() && body.len() <= 100_000,
        "release notes must be bounded and nonempty"
    );
    Ok(())
}

/// Generate the manifest for a complete, local release candidate.
pub fn assets(directory: &Path, selected: Version) -> Result<BTreeMap<String, String>> {
    let mut checksums = archive_assets(directory, selected)?;
    let manifest = checksum_manifest(&checksums);
    fs::write(directory.join("SHA256SUMS"), &manifest)?;
    checksums.insert("SHA256SUMS".into(), digest(manifest.as_bytes()));
    Ok(checksums)
}

fn assembled_assets(
    directory: &Path,
    selected: Version,
    notes: &Path,
) -> Result<BTreeMap<String, String>> {
    let mut checksums = archive_assets(directory, selected)?;
    let manifest = checksum_manifest(&checksums);
    ensure!(
        fs::read_to_string(directory.join("SHA256SUMS"))? == manifest,
        "candidate checksum manifest differs from its archives"
    );
    checked_notes(notes)?;
    checksums.insert("SHA256SUMS".into(), digest(manifest.as_bytes()));
    Ok(checksums)
}

/// Assemble one complete candidate without contacting GitHub.
pub fn assemble(directory: &Path, selected: Version, notes: &Path) -> Result<usize> {
    let generated = assets(directory, selected)?;
    let checked = assembled_assets(directory, selected, notes)?;
    ensure!(
        generated == checked,
        "assembled candidate verification disagrees with its generated manifest"
    );
    Ok(checked.len())
}
async fn find_release(api: &GitHub, selected: Version) -> Result<Option<Value>> {
    let tag = format!("v{selected}");
    for page in 1..=100 {
        let data = api
            .required(
                Method::GET,
                &api.path(&format!("releases?per_page=100&page={page}")),
                None,
            )
            .await?;
        let rows = data
            .as_array()
            .context("release listing must be an array")?;
        let matches: Vec<_> = rows
            .iter()
            .filter(|row| row["tag_name"].as_str() == Some(&tag))
            .collect();
        ensure!(matches.len() <= 1, "multiple releases use selected tag");
        if let Some(found) = matches.first() {
            return Ok(Some((*found).clone()));
        }
        if rows.len() < 100 {
            return Ok(None);
        }
    }
    bail!("release listing exceeded pagination limit")
}
fn remote_assets(
    release: &Value,
    checksums: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>> {
    let mut seen = BTreeSet::new();
    for item in release["assets"]
        .as_array()
        .context("missing release assets")?
    {
        let name = item["name"].as_str().context("missing asset name")?;
        let digest = checksums
            .get(name)
            .context("unexpected existing release asset")?;
        ensure!(
            seen.insert(name.to_owned())
                && item["digest"].as_str() == Some(&format!("sha256:{digest}")),
            "existing release asset differs from verified checksums"
        );
    }
    Ok(seen)
}
async fn published_url(api: &GitHub, release: &Value, selected: Version) -> Result<String> {
    let listed = release["assets"]
        .as_array()
        .context("missing release assets")?;
    let manifest_asset = listed
        .iter()
        .find(|item| item["name"] == "SHA256SUMS")
        .context("published release is incomplete")?;
    let id = manifest_asset["id"]
        .as_u64()
        .context("missing checksum asset ID")?;
    let bytes = api.checksum_manifest(id).await?;
    let text = std::str::from_utf8(&bytes)?;
    ensure!(
        text.lines().count() == TARGETS.len(),
        "published checksum manifest is incomplete"
    );
    let mut checksums = BTreeMap::new();
    for target in TARGETS {
        let name = crate::archive::archive_name(&selected.to_string(), target)?;
        checksums.insert(
            name.clone(),
            crate::archive::expected_digest(&bytes, &name)?,
        );
    }
    checksums.insert("SHA256SUMS".into(), digest(&bytes));
    ensure!(
        remote_assets(release, &checksums)? == checksums.keys().cloned().collect(),
        "published release is incomplete"
    );
    Ok(release["html_url"]
        .as_str()
        .context("missing release URL")?
        .to_owned())
}
pub async fn publish(
    api: &GitHub,
    directory: &Path,
    selected: Version,
    candidate: &str,
    notes: &Path,
) -> Result<String> {
    checked_sha(candidate)?;
    let tag = tag_commit(api, selected).await?;
    if let Some(existing) = &tag {
        ensure!(
            existing == candidate,
            "existing release tag points to a different commit"
        );
    }
    let existing = find_release(api, selected).await?;
    let marker = format!("<!-- kuru-release-sha: {candidate} -->");
    if let Some(release) = &existing {
        ensure!(
            release["body"]
                .as_str()
                .is_some_and(|body| body.contains(&marker)),
            "existing release does not belong to this release commit"
        );
        if release["draft"].as_bool() == Some(false) {
            ensure!(
                tag.as_deref() == Some(candidate),
                "published release tag is missing"
            );
            return published_url(api, release, selected).await;
        }
        ensure!(
            release["draft"].as_bool() == Some(true),
            "missing release draft state"
        );
    }
    let checksums = assembled_assets(directory, selected, notes)?;
    let body = fs::read_to_string(notes)?;
    let present = if let Some(release) = &existing {
        remote_assets(release, &checksums)?
    } else {
        BTreeSet::new()
    };
    ensure_tag(api, selected, candidate).await?;
    let release = if let Some(release) = existing {
        release
    } else {
        api.required(Method::POST, &api.path("releases"), Some(json!({"tag_name":format!("v{selected}"),"target_commitish":candidate,"name":format!("v{selected}"),"body":format!("{}\n\n{marker}\n",body.trim()),"draft":true,"prerelease":false}))).await?
    };
    let id = release["id"]
        .as_u64()
        .context("missing numeric release ID")?;
    for name in checksums.keys().filter(|name| !present.contains(*name)) {
        let current = api
            .required(Method::GET, &api.path(&format!("releases/{id}")), None)
            .await?;
        ensure!(
            current["draft"].as_bool() == Some(true),
            "draft was published concurrently; refusing further uploads"
        );
        api.upload(id, &directory.join(name)).await?;
    }
    let current = api
        .required(Method::GET, &api.path(&format!("releases/{id}")), None)
        .await?;
    ensure!(
        current["draft"].as_bool() == Some(true)
            && remote_assets(&current, &checksums)? == checksums.keys().cloned().collect(),
        "draft is incomplete; release remains unpublished"
    );
    let published = api
        .required(
            Method::PATCH,
            &api.path(&format!("releases/{id}")),
            // Let GitHub select latest by version/date; recovering an older
            // draft must not unconditionally promote it above newer releases.
            Some(json!({"draft":false,"make_latest":"legacy"})),
        )
        .await?;
    Ok(published["html_url"]
        .as_str()
        .context("missing release URL")?
        .to_owned())
}
