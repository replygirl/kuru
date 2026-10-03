//! Public GitHub release metadata and the authenticated previous release.

use crate::{archive, release};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Client, RequestBuilder, Response, Url,
    header::{AUTHORIZATION, HeaderValue, LINK},
    redirect::Policy,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
};

pub(crate) const REPOSITORY: &str = "replygirl/kuru";
pub(crate) const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const METADATA_HOST: &str = "api.github.com";
const MANIFEST_LIMIT: usize = 64 * 1024;
/// GitHub lists 30 releases per page; this bounds the walk at 3,000 releases.
/// Reaching it leaves the page chain incomplete, which the no-predecessor
/// branch refuses.
const MAX_LISTING_PAGES: usize = 100;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PublishedRelease {
    pub(crate) tag_name: String,
    pub(crate) draft: bool,
    pub(crate) prerelease: bool,
    pub(crate) assets: Vec<ReleaseAsset>,
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

/// Normalize an optional GitHub token taken from the caller's environment.
/// Unset or empty means unauthenticated; anything else must be a single
/// printable token so it can only ever become one bearer header value.
pub fn checked_token(raw: Option<OsString>) -> Result<Option<String>> {
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let token = raw
        .into_string()
        .ok()
        .filter(|token| token.bytes().all(|byte| byte.is_ascii_graphic()))
        .context("GITHUB_TOKEN must be a single printable ASCII token")?;
    Ok(Some(token))
}

/// Public GitHub reads. An optional token authenticates only metadata
/// requests to the REST API host, which raises its rate limit; asset downloads
/// are always anonymous. This type deliberately has no `Debug`.
pub(crate) struct PublicGitHub {
    client: Client,
    authorization: Option<HeaderValue>,
}

impl PublicGitHub {
    pub(crate) fn new(token: Option<&str>) -> Result<Self> {
        let authorization = token
            .map(|token| {
                let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                    .context("GitHub token is not a valid header value")?;
                value.set_sensitive(true);
                Ok::<_, anyhow::Error>(value)
            })
            .transpose()?;
        Ok(Self::with_client(published_client()?, authorization))
    }

    fn with_client(client: Client, authorization: Option<HeaderValue>) -> Self {
        Self {
            client,
            authorization,
        }
    }

    fn request(&self, url: &str) -> Result<RequestBuilder> {
        let url = Url::parse(url)?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "published release URL must be credential-free HTTPS"
        );
        Ok(self.checked_request(url))
    }

    /// A request for a URL that is either checked by [`Self::request`] or is
    /// the fixed release-listing page URL built by [`Self::release_page`].
    fn checked_request(&self, url: Url) -> RequestBuilder {
        let metadata = url.host_str() == Some(METADATA_HOST) && url.port().is_none();
        let mut request = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(authorization) = self.authorization.as_ref().filter(|_| metadata) {
            request = request.header(AUTHORIZATION, authorization.clone());
        }
        request
    }

    pub(crate) async fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let response = send(self.request(url)?).await?;
        bounded_body(response, limit).await
    }

    /// One page (from 1) of the repository's release listing and whether
    /// GitHub names the page after it. The page URL is always built here from
    /// the fixed listing path; a server-supplied `Link` URL is never requested,
    /// it only decides whether the next page exists.
    async fn release_page(&self, number: usize) -> Result<(Vec<PublishedRelease>, bool)> {
        let response = send(self.checked_request(listing_page_url(number)?)).await?;
        let link = response
            .headers()
            .get(LINK)
            .map(|value| {
                value
                    .to_str()
                    .map(str::to_owned)
                    .context("GitHub release listing Link header is not ASCII")
            })
            .transpose()?;
        let bytes = bounded_body(response, METADATA_LIMIT).await?;
        let releases = serde_json::from_slice(&bytes).context("invalid GitHub release metadata")?;
        Ok((releases, next_page(link.as_deref(), number)?))
    }

    async fn json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let bytes = self
            .get(
                &format!("https://{METADATA_HOST}/repos/{REPOSITORY}/{path}"),
                METADATA_LIMIT,
            )
            .await?;
        serde_json::from_slice(&bytes).context("invalid GitHub release metadata")
    }

    pub(crate) async fn release(&self, version: &str) -> Result<PublishedRelease> {
        self.json(&format!("releases/tags/v{version}")).await
    }

    pub(crate) async fn tag_commit(&self, version: &str) -> Result<String> {
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

/// The production client for published release reads. There is deliberately
/// no total timeout: connection setup and every read are bounded, and each
/// caller's byte limit bounds how many reads a response can take.
fn published_client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::limited(5))
        .connect_timeout(archive::CONNECT_TIMEOUT)
        .read_timeout(archive::READ_IDLE_TIMEOUT)
        .user_agent("kuru-published-release-client")
        .build()?)
}

/// Send one published release request, naming that phase if it fails. An
/// unsuccessful status keeps its own message.
async fn send(request: RequestBuilder) -> Result<Response> {
    Ok(request
        .send()
        .await
        .context("send published release request")?
        .error_for_status()?)
}

/// The fixed URL of one release-listing page.
fn listing_page_url(number: usize) -> Result<Url> {
    let mut url = Url::parse(&format!(
        "https://{METADATA_HOST}/repos/{REPOSITORY}/releases"
    ))?;
    url.query_pairs_mut()
        .append_pair("page", &number.to_string());
    Ok(url)
}

async fn bounded_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= limit as u64),
        "published response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("read published release response body")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= limit,
            "published response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Whether a release-listing `Link` header names the page after `current`.
/// Every entry must be a well-formed `<URL>; rel="..."` link, and a `next`
/// relation must name exactly that page on the API host. Anything else breaks
/// the page chain and fails instead of being read as the last page.
fn next_page(link: Option<&str>, current: usize) -> Result<bool> {
    let Some(link) = link else {
        return Ok(false);
    };
    let mut next = None;
    for entry in link.split(',') {
        let mut parts = entry.split(';');
        let url = parts
            .next()
            .unwrap_or_default()
            .trim()
            .strip_prefix('<')
            .and_then(|reference| reference.strip_suffix('>'))
            .and_then(|reference| Url::parse(reference).ok());
        let relation = parts.find_map(|parameter| {
            parameter
                .trim()
                .strip_prefix("rel=\"")
                .and_then(|relation| relation.strip_suffix('"'))
        });
        let (Some(url), Some(relation)) = (url, relation) else {
            bail!("GitHub release listing Link header is malformed");
        };
        if relation == "next" {
            ensure!(
                next.replace(url).is_none(),
                "GitHub release listing names more than one next page"
            );
        }
    }
    let Some(url) = next else {
        return Ok(false);
    };
    let expected = current
        .checked_add(1)
        .context("release listing page overflows")?
        .to_string();
    let pages: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "page")
        .map(|(_, value)| value.into_owned())
        .collect();
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some(METADATA_HOST)
            && pages == [expected.as_str()],
        "GitHub release listing's next page is not page {expected} on {METADATA_HOST}"
    );
    Ok(true)
}

/// The published stable release immediately preceding a candidate for one
/// target, with its checksum manifest and that target's core archive already
/// authenticated. Its own updater is what existing installations of that
/// target run against the candidate. Acceptance from it is a floor, not the
/// compatibility policy: updating from any installed release must remain
/// possible.
pub struct PreviousRelease {
    pub version: String,
    pub target: &'static str,
    pub manifest: Vec<u8>,
    pub archive: Vec<u8>,
    /// The paired shell-support envelope, present exactly when the previous
    /// release publishes one for this target.
    pub support: Option<Vec<u8>>,
    /// Newer stable releases inspected first whose listing and `SHA256SUMS`
    /// agree that they publish no archive for this target, newest first.
    pub skipped: Vec<String>,
}

/// The previous-release decision for one target.
pub enum Predecessor {
    /// A published stable release carries the target; its updater must be
    /// exercised against the candidate.
    Release(PreviousRelease),
    /// No published stable release carries the target. Every page of the
    /// release listing was read, and each inspected release's listing and
    /// authenticated `SHA256SUMS` agree that it lacks the target's archive.
    None {
        target: &'static str,
        candidate: String,
        /// Every stable release older than the candidate, newest first.
        inspected: Vec<String>,
        /// How much of the listing was read: `across N releases, all pages`.
        horizon: String,
    },
}

impl Predecessor {
    pub fn target(&self) -> &'static str {
        match self {
            Self::Release(previous) => previous.target,
            Self::None { target, .. } => target,
        }
    }

    /// The job-log evidence line for the no-predecessor branch, which is the
    /// acceptance evidence on a target's first release.
    pub fn evidence(&self) -> Option<String> {
        let Self::None {
            target,
            inspected,
            horizon,
            ..
        } = self
        else {
            return None;
        };
        let inspected = inspected
            .iter()
            .map(|version| format!("v{version}"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "no predecessor for {target}: inspected {inspected} {horizon}"
        ))
    }
}

/// The core archive and shell-support envelope names a release publishes for
/// one target: `.tar.gz` on Unix targets and `.zip` on Windows.
fn target_assets(version: &str, target: &str) -> Result<(String, String)> {
    Ok((
        archive::archive_name(version, target)?,
        crate::shell_support::archive_name(version, target)?,
    ))
}

/// Where predecessor selection reads the release listing and each inspected
/// release's authenticated checksum manifest: public GitHub in production and
/// in-memory fixtures in tests.
trait ReleaseSource {
    /// Page `number` (from 1) of the release listing and whether a next page
    /// exists.
    async fn page(&mut self, number: usize) -> Result<(Vec<PublishedRelease>, bool)>;
    /// The listed release's `SHA256SUMS`, authenticated against its listed
    /// digest before it is returned.
    async fn manifest(&mut self, listed: &PublishedRelease) -> Result<Vec<u8>>;
}

struct GitHubReleases<'a>(&'a PublicGitHub);

impl ReleaseSource for GitHubReleases<'_> {
    async fn page(&mut self, number: usize) -> Result<(Vec<PublishedRelease>, bool)> {
        self.0.release_page(number).await
    }

    async fn manifest(&mut self, listed: &PublishedRelease) -> Result<Vec<u8>> {
        let version = listed
            .tag_name
            .strip_prefix('v')
            .context("published release tag is not vX.Y.Z")?;
        let digest = listed_digest(listed, version, "SHA256SUMS")?;
        let manifest = self
            .0
            .get(
                &format!("https://github.com/{REPOSITORY}/releases/download/v{version}/SHA256SUMS"),
                MANIFEST_LIMIT,
            )
            .await?;
        verify_manifest(&manifest, &digest)?;
        Ok(manifest)
    }
}

/// Resolve and download the release preceding `candidate` for `target` from
/// public GitHub, or establish that no published release carries `target`.
///
/// Nothing is pinned: the release is selected at run time by
/// [`select_previous`] over the release listing, following every listing page
/// before concluding that no predecessor exists. `token` authenticates only
/// the release listing; every asset is read anonymously over HTTPS from its
/// immutable version path, and no bytes are returned until the archive and any
/// paired shell-support envelope match that release's own `SHA256SUMS` and
/// every file matches GitHub's asset digest.
pub async fn previous_release(
    candidate: &str,
    target: &str,
    token: Option<&str>,
) -> Result<Predecessor> {
    let candidate = candidate.parse::<release::Version>()?;
    let target = crate::targets::find(target)?.triple;
    let github = PublicGitHub::new(token)?;
    let (selection, releases) = resolve(&mut GitHubReleases(&github), candidate, target).await?;
    let (version, manifest, skipped) = match selection {
        Selection::Release {
            version,
            manifest,
            skipped,
        } => (version.to_string(), manifest, skipped),
        Selection::None { inspected } => {
            return Ok(no_predecessor(
                target,
                candidate,
                &inspected,
                releases.len(),
            ));
        }
        Selection::Incomplete => bail!("release listing ended before selection completed"),
    };
    let listed = releases
        .iter()
        .find(|release| release.tag_name == format!("v{version}"))
        .context("selected previous release is absent from its listing")?;
    let (archive_name, support_name) = target_assets(&version, target)?;
    // Selection received this manifest authenticated from its source; check it
    // against the listing again before trusting it for the archive.
    verify_manifest(&manifest, &listed_digest(listed, &version, "SHA256SUMS")?)?;
    let archive_digest = listed_digest(listed, &version, &archive_name)?;
    let download_base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
    let archive = github
        .get(
            &format!("{download_base}/{archive_name}"),
            archive::MAX_ARCHIVE_BYTES,
        )
        .await?;
    verify_listed_asset(&manifest, &archive, &archive_name, &archive_digest)?;
    let support = if publishes_asset(listed, &manifest, &support_name) {
        // Either source naming the envelope requires both to agree on it.
        let support_digest = listed_digest(listed, &version, &support_name)?;
        let support = github
            .get(
                &format!("{download_base}/{support_name}"),
                crate::shell_support::MAX_ENVELOPE_BYTES,
            )
            .await?;
        verify_listed_asset(&manifest, &support, &support_name, &support_digest)?;
        Some(support)
    } else {
        None
    };
    Ok(Predecessor::Release(PreviousRelease {
        version,
        target,
        manifest,
        archive,
        support,
        skipped: skipped.iter().map(ToString::to_string).collect(),
    }))
}

fn no_predecessor(
    target: &'static str,
    candidate: release::Version,
    inspected: &[release::Version],
    releases: usize,
) -> Predecessor {
    Predecessor::None {
        target,
        candidate: candidate.to_string(),
        inspected: inspected.iter().map(ToString::to_string).collect(),
        horizon: format!("across {releases} releases, all pages"),
    }
}

/// The outcome of target-scoped selection over the listing read so far.
#[derive(Debug, PartialEq)]
enum Selection {
    /// The greatest older stable release whose listing and manifest both name
    /// the target's archive, with its authenticated manifest and the newer
    /// releases skipped because both sources agree they lack it.
    Release {
        version: release::Version,
        manifest: Vec<u8>,
        skipped: Vec<release::Version>,
    },
    /// The listing is complete and no stable older release carries the target.
    None { inspected: Vec<release::Version> },
    /// Nothing read so far carries the target and more listing pages exist.
    Incomplete,
}

/// Read the release listing page by page until selection finds the target's
/// predecessor or every page is exhausted, fetching each inspected release's
/// authenticated manifest once. An unreadable or over-long page chain fails
/// rather than letting a partial listing establish that no predecessor exists.
async fn resolve(
    source: &mut impl ReleaseSource,
    candidate: release::Version,
    target: &str,
) -> Result<(Selection, Vec<PublishedRelease>)> {
    let (mut releases, mut more) = source.page(1).await?;
    let mut pages = 1;
    let mut manifests: HashMap<String, Vec<u8>> = HashMap::new();
    loop {
        let mut missing = None;
        let selection = select_previous(
            &releases,
            !more,
            candidate,
            target,
            &mut |listed: &PublishedRelease| {
                if let Some(manifest) = manifests.get(&listed.tag_name) {
                    return Ok(manifest.clone());
                }
                missing = Some(listed.tag_name.clone());
                bail!("SHA256SUMS for {} is not fetched yet", listed.tag_name)
            },
        );
        if let Some(tag) = missing {
            let listed = releases
                .iter()
                .find(|listed| listed.tag_name == tag)
                .context("inspected release is absent from its listing")?;
            let manifest = source.manifest(listed).await?;
            manifests.insert(tag, manifest);
            continue;
        }
        match selection? {
            Selection::Incomplete => {
                ensure!(
                    pages < MAX_LISTING_PAGES,
                    "incomplete release listing: more than {MAX_LISTING_PAGES} pages, so no predecessor for {target} cannot be established"
                );
                pages += 1;
                let (page, next) = source.page(pages).await.with_context(|| {
                    format!(
                        "incomplete release listing: page {pages} could not be read, so no predecessor for {target} cannot be established"
                    )
                })?;
                ensure!(
                    !page.is_empty(),
                    "incomplete release listing: page {pages} is empty although GitHub named it"
                );
                releases.extend(page);
                more = next;
            }
            selection => return Ok((selection, releases)),
        }
    }
}

/// Target-scoped selection of the greatest older stable release carrying
/// `target`, over the listing read so far and an injected provider of
/// authenticated manifests. The candidate itself is skipped, which covers a
/// recovered Release run. Fail closed when a stable release tag is not
/// canonical `vX.Y.Z`, is newer than the candidate, or appears twice, and when
/// a release's listing and `SHA256SUMS` disagree about the target's archive.
/// No stable older release at all remains an error once the listing is
/// complete: a project's first release differs from a target's first release.
fn select_previous(
    releases: &[PublishedRelease],
    complete: bool,
    candidate: release::Version,
    target: &str,
    manifest: &mut dyn FnMut(&PublishedRelease) -> Result<Vec<u8>>,
) -> Result<Selection> {
    let mut older = Vec::new();
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
        older.push((version, listed));
    }
    older.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    ensure!(
        older.windows(2).all(|pair| pair[0].0 != pair[1].0),
        "the release listing names a published stable release more than once"
    );
    if older.is_empty() {
        ensure!(
            !complete,
            "no published stable release precedes v{candidate}"
        );
        return Ok(Selection::Incomplete);
    }
    let mut inspected = Vec::new();
    for (version, listed) in older {
        let name = archive::archive_name(&version.to_string(), target)?;
        let manifest = manifest(listed)?;
        if listed.assets.iter().any(|asset| asset.name == name) {
            archive::expected_digest(&manifest, &name).with_context(|| {
                format!(
                    "previous release v{version} lists {name} inconsistently: its SHA256SUMS does not name it exactly once"
                )
            })?;
            return Ok(Selection::Release {
                version,
                manifest,
                skipped: inspected,
            });
        }
        ensure!(
            !publishes_asset(listed, &manifest, &name),
            "previous release v{version} omits {name} inconsistently: its SHA256SUMS names it"
        );
        inspected.push(version);
    }
    Ok(if complete {
        Selection::None { inspected }
    } else {
        Selection::Incomplete
    })
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

fn publishes_asset(listed: &PublishedRelease, manifest: &[u8], name: &str) -> bool {
    listed.assets.iter().any(|asset| asset.name == name)
        || String::from_utf8_lossy(manifest).lines().any(|line| {
            line.split_once(' ')
                .and_then(|(_, listed)| listed.strip_prefix(' ').or(listed.strip_prefix('*')))
                == Some(name)
        })
}

fn verify_manifest(manifest: &[u8], manifest_digest: &str) -> Result<()> {
    ensure!(
        archive::digest(manifest) == manifest_digest,
        "previous release SHA256SUMS differs from its GitHub asset digest"
    );
    Ok(())
}

fn verify_listed_asset(
    manifest: &[u8],
    bytes: &[u8],
    name: &str,
    listed_digest: &str,
) -> Result<()> {
    let expected = archive::expected_digest(manifest, name)?;
    ensure!(
        archive::digest(bytes) == expected,
        "previous release {name} differs from its SHA256SUMS"
    );
    ensure!(
        expected == listed_digest,
        "previous release SHA256SUMS and GitHub disagree on {name}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";
    /// A catalog target that no fixture release publishes, standing in for a
    /// newly added target such as `aarch64-pc-windows-msvc`, whose evidence
    /// line is checked directly below.
    const NEW_TARGET: &str = "aarch64-unknown-linux-gnu";

    fn listed(tag: &str, draft: bool, prerelease: bool) -> PublishedRelease {
        PublishedRelease {
            tag_name: tag.into(),
            draft,
            prerelease,
            assets: Vec::new(),
        }
    }

    fn named_asset(version: &str, name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"
            ),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }
    }

    /// A stable release whose listing names the core archive of each target.
    fn carrying(tag: &str, targets: &[&str]) -> PublishedRelease {
        let version = tag.strip_prefix('v').unwrap();
        let mut release = listed(tag, false, false);
        release.assets = std::iter::once("SHA256SUMS".to_owned())
            .chain(
                targets
                    .iter()
                    .map(|target| archive::archive_name(version, target).unwrap()),
            )
            .map(|name| named_asset(version, &name))
            .collect();
        release
    }

    /// A `SHA256SUMS` naming exactly the archives the listing names.
    fn agreeing_manifest(listed: &PublishedRelease) -> Result<Vec<u8>> {
        Ok(listed
            .assets
            .iter()
            .filter(|asset| asset.name != "SHA256SUMS")
            .map(|asset| format!("{}  {}\n", "a".repeat(64), asset.name))
            .collect::<String>()
            .into_bytes())
    }

    fn version(value: &str) -> release::Version {
        value.parse().unwrap()
    }

    fn versions(values: &[&str]) -> Vec<release::Version> {
        values.iter().map(|value| version(value)).collect()
    }

    /// Selection over a complete listing whose manifests agree with it.
    fn select(releases: &[PublishedRelease], candidate: &str, target: &str) -> Result<Selection> {
        select_previous(
            releases,
            true,
            version(candidate),
            target,
            &mut agreeing_manifest,
        )
    }

    fn selected(selection: Selection) -> (release::Version, Vec<release::Version>) {
        match selection {
            Selection::Release {
                version, skipped, ..
            } => (version, skipped),
            other => panic!("expected a predecessor, got {other:?}"),
        }
    }

    /// An in-memory paginated listing; pages past `readable` fail to load.
    struct Memory {
        pages: Vec<Vec<PublishedRelease>>,
        readable: usize,
        page_reads: Vec<usize>,
        manifest_reads: Vec<String>,
    }

    impl Memory {
        fn new(pages: Vec<Vec<PublishedRelease>>) -> Self {
            Self {
                readable: pages.len(),
                pages,
                page_reads: Vec::new(),
                manifest_reads: Vec::new(),
            }
        }
    }

    impl ReleaseSource for Memory {
        async fn page(&mut self, number: usize) -> Result<(Vec<PublishedRelease>, bool)> {
            self.page_reads.push(number);
            ensure!(number <= self.readable, "page {number} is unavailable");
            Ok((self.pages[number - 1].clone(), number < self.pages.len()))
        }

        async fn manifest(&mut self, listed: &PublishedRelease) -> Result<Vec<u8>> {
            self.manifest_reads.push(listed.tag_name.clone());
            agreeing_manifest(listed)
        }
    }

    #[test]
    fn previous_release_is_the_greatest_older_stable_release() {
        let releases = [
            listed("v0.10.0", true, false),
            listed("v0.9.1-rc.1", false, true),
            carrying("v0.8.0", &[WINDOWS_TARGET]),
            carrying("v0.9.0", &[WINDOWS_TARGET]),
            carrying("v0.4.2", &[WINDOWS_TARGET]),
        ];
        assert_eq!(
            selected(select(&releases, "0.10.0", WINDOWS_TARGET).unwrap()).0,
            version("0.9.0")
        );

        // A recovered run whose candidate is already public uses the one before.
        let recovered = [
            carrying("v0.10.0", &[WINDOWS_TARGET]),
            carrying("v0.9.0", &[WINDOWS_TARGET]),
        ];
        assert_eq!(
            selected(select(&recovered, "0.10.0", WINDOWS_TARGET).unwrap()).0,
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
            (
                vec![
                    carrying("v0.9.0", &[WINDOWS_TARGET]),
                    carrying("v0.9.0", &[WINDOWS_TARGET]),
                ],
                "0.10.0",
            ),
        ] {
            assert!(
                select(&releases, candidate, WINDOWS_TARGET).is_err(),
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
        verify_manifest(&manifest, &manifest_digest).unwrap();
        verify_listed_asset(&manifest, &archive_bytes, &name, &archive_digest).unwrap();

        let other = "f".repeat(64);
        assert!(verify_manifest(&manifest, &other).is_err());
        assert!(verify_listed_asset(&manifest, b"altered", &name, &archive_digest).is_err());
        assert!(verify_listed_asset(&manifest, &archive_bytes, &name, &other).is_err());
        let foreign = format!("{archive_digest}  kuru-0.9.0-other.zip\n").into_bytes();
        assert!(verify_listed_asset(&foreign, &archive_bytes, &name, &archive_digest).is_err());

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
    fn previous_support_envelope_is_detected_and_authenticated() {
        let core = archive::archive_name("1.0.0", WINDOWS_TARGET).unwrap();
        let support = crate::shell_support::archive_name("1.0.0", WINDOWS_TARGET).unwrap();
        let envelope = b"support envelope".to_vec();
        let envelope_digest = archive::digest(&envelope);
        let unmarked = format!("{}  {core}\n", "a".repeat(64)).into_bytes();
        let marked =
            format!("{}  {core}\n{envelope_digest}  {support}\n", "a".repeat(64)).into_bytes();
        let asset = |name: &str| ReleaseAsset {
            name: name.into(),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/v1.0.0/{name}"
            ),
            digest: Some(format!("sha256:{envelope_digest}")),
        };
        let mut release = listed("v1.0.0", false, false);
        release.assets = vec![asset(&core)];

        // An executable-only release names no envelope in either source.
        assert!(!publishes_asset(&release, &unmarked, &support));
        // Naming it in either source requires it; a listing without it (or a
        // manifest without it) then fails authentication instead of skipping.
        assert!(publishes_asset(&release, &marked, &support));
        assert!(listed_digest(&release, "1.0.0", &support).is_err());
        release.assets.push(asset(&support));
        assert!(publishes_asset(&release, &unmarked, &support));
        assert!(verify_listed_asset(&unmarked, &envelope, &support, &envelope_digest).is_err());
        let binary_marked =
            format!("{}  {core}\n{envelope_digest} *{support}\n", "a".repeat(64)).into_bytes();
        assert!(publishes_asset(
            &listed("v1.0.0", false, false),
            &binary_marked,
            &support
        ));

        let digest = listed_digest(&release, "1.0.0", &support).unwrap();
        verify_listed_asset(&marked, &envelope, &support, &digest).unwrap();
        assert!(verify_listed_asset(&marked, b"altered envelope", &support, &digest).is_err());
        assert!(verify_listed_asset(&marked, &envelope, &support, &"f".repeat(64)).is_err());
        let substituted = format!(
            "{}  {core}\n{}  {support}\n",
            "a".repeat(64),
            "b".repeat(64)
        )
        .into_bytes();
        assert!(verify_listed_asset(&substituted, &envelope, &support, &digest).is_err());
    }

    #[test]
    fn previous_release_assets_are_named_per_target() {
        for target in crate::targets::CATALOG {
            let (core, support) = target_assets("0.9.0", target.triple).unwrap();
            // Both Windows targets ship ZIP archives; every other target tar.gz.
            let extension = if target.triple.ends_with("-pc-windows-msvc") {
                "zip"
            } else {
                "tar.gz"
            };
            assert_eq!(core, format!("kuru-0.9.0-{}.{extension}", target.triple));
            assert_eq!(
                support,
                format!("kuru-0.9.0-{}-shell-support.{extension}", target.triple)
            );
        }
        assert!(target_assets("0.9.0", "riscv64gc-unknown-linux-gnu").is_err());
        assert!(target_assets("0.9", "aarch64-apple-darwin").is_err());
    }

    #[test]
    fn synthetic_next_patch_candidate_selects_the_published_workspace_release() {
        // CI packages main's tree under the next patch version, so the
        // published release matching the workspace version is the previous one.
        let releases = [
            carrying("v0.9.0", &[WINDOWS_TARGET]),
            carrying("v0.8.0", &[WINDOWS_TARGET]),
        ];
        assert_eq!(
            selected(select(&releases, "0.9.1", WINDOWS_TARGET).unwrap()).0,
            version("0.9.0")
        );
        // A branch older than the latest publication fails closed; rebase it.
        let ahead = [carrying("v0.10.0", &[WINDOWS_TARGET])];
        assert!(select(&ahead, "0.9.1", WINDOWS_TARGET).is_err());
    }

    #[test]
    fn predecessor_is_the_greatest_older_release_carrying_the_target() {
        let releases = [
            carrying("v0.9.0", &[WINDOWS_TARGET]),
            carrying("v0.10.0", &[WINDOWS_TARGET]),
        ];
        let mut fetched = Vec::new();
        let selection = select_previous(
            &releases,
            true,
            version("0.10.1"),
            WINDOWS_TARGET,
            &mut |listed: &PublishedRelease| {
                fetched.push(listed.tag_name.clone());
                agreeing_manifest(listed)
            },
        )
        .unwrap();
        assert_eq!(
            selection,
            Selection::Release {
                version: version("0.10.0"),
                manifest: agreeing_manifest(&releases[1]).unwrap(),
                skipped: Vec::new(),
            }
        );
        // Only the selected release's manifest is read; it is authenticated by
        // the provider and handed on with the selection.
        assert_eq!(fetched, ["v0.10.0"]);
    }

    #[test]
    fn predecessor_skips_releases_without_the_target_when_a_later_one_exists() {
        let releases = [
            carrying("v0.10.0", &[WINDOWS_TARGET]),
            carrying("v0.9.0", &[WINDOWS_TARGET, NEW_TARGET]),
            carrying("v0.8.0", &[WINDOWS_TARGET, NEW_TARGET]),
        ];
        assert_eq!(
            selected(select(&releases, "0.11.0", NEW_TARGET).unwrap()),
            (version("0.9.0"), versions(&["0.10.0"]))
        );
    }

    #[test]
    fn no_predecessor_when_no_stable_release_carries_the_target() {
        let mut draft = carrying("v0.10.5", &[NEW_TARGET]);
        draft.draft = true;
        let mut prerelease = listed("v0.10.2-rc.1", false, true);
        prerelease.assets = vec![named_asset(
            "0.10.2-rc.1",
            &format!("kuru-0.10.2-rc.1-{NEW_TARGET}.tar.gz"),
        )];
        let releases = [
            draft,
            prerelease,
            carrying("v0.9.0", &[WINDOWS_TARGET]),
            carrying("v0.10.0", &[WINDOWS_TARGET]),
        ];
        let mut fetched = Vec::new();
        let selection = select_previous(
            &releases,
            true,
            version("0.11.0"),
            NEW_TARGET,
            &mut |listed: &PublishedRelease| {
                fetched.push(listed.tag_name.clone());
                agreeing_manifest(listed)
            },
        )
        .unwrap();
        assert_eq!(
            selection,
            Selection::None {
                inspected: versions(&["0.10.0", "0.9.0"])
            }
        );
        // Draft and prerelease entries are never inspected.
        assert_eq!(fetched, ["v0.10.0", "v0.9.0"]);

        // A project's first release is not a target's first release: with no
        // stable older release at all, the existing error stands.
        let error = select(&releases[..2], "0.11.0", NEW_TARGET)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "no published stable release precedes v0.11.0");
    }

    #[test]
    fn no_predecessor_requires_the_manifest_to_agree() {
        let new_archive = archive::archive_name("0.10.0", NEW_TARGET).unwrap();
        // The listing omits the target's archive but SHA256SUMS names it.
        let omitted = [carrying("v0.10.0", &[WINDOWS_TARGET])];
        let error = select_previous(
            &omitted,
            true,
            version("0.11.0"),
            NEW_TARGET,
            &mut |listed: &PublishedRelease| {
                let mut manifest = agreeing_manifest(listed)?;
                manifest.extend(format!("{}  {new_archive}\n", "b".repeat(64)).bytes());
                Ok(manifest)
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("omits") && error.contains("inconsistently"),
            "{error}"
        );

        // The listing names the target's archive but SHA256SUMS does not.
        let listed_only = [carrying("v0.10.0", &[WINDOWS_TARGET, NEW_TARGET])];
        let error = select_previous(
            &listed_only,
            true,
            version("0.11.0"),
            NEW_TARGET,
            &mut |_: &PublishedRelease| {
                Ok(format!(
                    "{}  {}\n",
                    "a".repeat(64),
                    archive::archive_name("0.10.0", WINDOWS_TARGET).unwrap()
                )
                .into_bytes())
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("lists") && error.contains("inconsistently"),
            "{error}"
        );

        // A manifest that fails authentication stops selection.
        let error = select_previous(
            &omitted,
            true,
            version("0.11.0"),
            NEW_TARGET,
            &mut |_: &PublishedRelease| {
                bail!("previous release SHA256SUMS differs from its GitHub asset digest")
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("differs from its GitHub asset digest"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn no_predecessor_follows_every_listing_page() {
        let first = vec![
            carrying("v0.10.0", &[WINDOWS_TARGET]),
            carrying("v0.9.0", &[WINDOWS_TARGET]),
        ];
        let second = vec![carrying("v0.8.0", &[WINDOWS_TARGET, NEW_TARGET])];

        // Only the second page carries the target: it is read and selected,
        // and each inspected manifest is fetched once.
        let mut source = Memory::new(vec![first.clone(), second.clone()]);
        let (selection, releases) = resolve(&mut source, version("0.11.0"), NEW_TARGET)
            .await
            .unwrap();
        assert_eq!(
            selected(selection),
            (version("0.8.0"), versions(&["0.10.0", "0.9.0"]))
        );
        assert_eq!(releases.len(), 3);
        assert_eq!(source.page_reads, [1, 2]);
        assert_eq!(source.manifest_reads, ["v0.10.0", "v0.9.0", "v0.8.0"]);

        // A predecessor on the first page needs no further page.
        let mut source = Memory::new(vec![first.clone(), second.clone()]);
        let (selection, _) = resolve(&mut source, version("0.11.0"), WINDOWS_TARGET)
            .await
            .unwrap();
        assert_eq!(selected(selection).0, version("0.10.0"));
        assert_eq!(source.page_reads, [1]);

        // An incomplete page chain cannot establish that no predecessor exists.
        let mut source = Memory::new(vec![first.clone(), second.clone()]);
        source.readable = 1;
        let error = resolve(&mut source, version("0.11.0"), NEW_TARGET)
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("incomplete release listing: page 2 could not be read"),
            "{error:#}"
        );

        // A named page that turns out empty breaks the chain too.
        let mut source = Memory::new(vec![first.clone(), Vec::new()]);
        let error = resolve(&mut source, version("0.11.0"), NEW_TARGET)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("page 2 is empty"), "{error}");

        // The complete chain without the target is the no-predecessor branch.
        let mut source = Memory::new(vec![first, vec![carrying("v0.8.0", &[WINDOWS_TARGET])]]);
        let (selection, releases) = resolve(&mut source, version("0.11.0"), NEW_TARGET)
            .await
            .unwrap();
        assert_eq!(
            selection,
            Selection::None {
                inspected: versions(&["0.10.0", "0.9.0", "0.8.0"])
            }
        );
        assert_eq!(releases.len(), 3);
    }

    #[tokio::test]
    async fn no_predecessor_prints_the_exact_evidence_line() {
        let mut draft = listed("v0.11.0", true, false);
        draft.assets = vec![named_asset("0.11.0", "SHA256SUMS")];
        let mut source = Memory::new(vec![
            vec![draft, carrying("v0.10.0", &[WINDOWS_TARGET])],
            vec![carrying("v0.9.0", &[WINDOWS_TARGET])],
        ]);
        let (selection, releases) = resolve(&mut source, version("0.11.0"), NEW_TARGET)
            .await
            .unwrap();
        let Selection::None { inspected } = selection else {
            panic!("expected no predecessor, got {selection:?}");
        };
        let predecessor = no_predecessor(NEW_TARGET, version("0.11.0"), &inspected, releases.len());
        assert_eq!(predecessor.target(), NEW_TARGET);
        assert_eq!(
            predecessor.evidence().unwrap(),
            "no predecessor for aarch64-unknown-linux-gnu: inspected v0.10.0, v0.9.0 across 3 releases, all pages"
        );

        let arm64 = Predecessor::None {
            target: "aarch64-pc-windows-msvc",
            candidate: "0.10.0".into(),
            inspected: vec!["0.9.0".into(), "0.8.0".into()],
            horizon: "across 16 releases, all pages".into(),
        };
        assert_eq!(
            arm64.evidence().unwrap(),
            "no predecessor for aarch64-pc-windows-msvc: inspected v0.9.0, v0.8.0 across 16 releases, all pages"
        );

        let release = Predecessor::Release(PreviousRelease {
            version: "0.9.0".into(),
            target: WINDOWS_TARGET,
            manifest: Vec::new(),
            archive: Vec::new(),
            support: None,
            skipped: Vec::new(),
        });
        assert_eq!(release.target(), WINDOWS_TARGET);
        assert!(release.evidence().is_none());
    }

    #[test]
    fn listing_pages_follow_only_the_exact_next_page() {
        let link = |page: &str, host: &str| {
            format!(
                "<https://{host}/repositories/1/releases?page={page}>; rel=\"next\", <https://{host}/repositories/1/releases?page=5>; rel=\"last\""
            )
        };
        assert!(!next_page(None, 1).unwrap());
        assert!(next_page(Some(&link("2", METADATA_HOST)), 1).unwrap());
        assert!(
            !next_page(
                Some("<https://api.github.com/repositories/1/releases?page=1>; rel=\"prev\""),
                2
            )
            .unwrap()
        );
        for (header, current) in [
            (link("3", METADATA_HOST), 1),
            (link("2", "example.invalid"), 1),
            (link("2&page=3", METADATA_HOST), 1),
            (
                "https://api.github.com/releases?page=2; rel=\"next\"".to_owned(),
                1,
            ),
            ("unrecognized".to_owned(), 1),
            (String::new(), 1),
            (
                "<https://api.github.com/repositories/1/releases?page=1>; rel=prev".to_owned(),
                2,
            ),
            (
                format!("{}, {}", link("2", METADATA_HOST), link("2", METADATA_HOST)),
                1,
            ),
            (link("2", METADATA_HOST).replace("https://", "http://"), 1),
        ] {
            assert!(next_page(Some(&header), current).is_err(), "{header}");
        }
    }

    #[test]
    fn optional_token_is_normalized_without_accepting_malformed_values() {
        assert_eq!(checked_token(None).unwrap(), None);
        assert_eq!(checked_token(Some(OsString::new())).unwrap(), None);
        assert_eq!(
            checked_token(Some("ghs_fake-token_123".into())).unwrap(),
            Some("ghs_fake-token_123".to_owned())
        );
        for malformed in ["two words", "line\nbreak", "tab\t", "é"] {
            assert!(
                checked_token(Some(malformed.into())).is_err(),
                "{malformed:?}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            assert!(checked_token(Some(OsString::from_vec(vec![0xff]))).is_err());
        }
    }

    #[test]
    fn token_is_sent_only_on_api_metadata_requests() {
        let metadata = format!("https://{METADATA_HOST}/repos/{REPOSITORY}/releases");
        let download =
            format!("https://github.com/{REPOSITORY}/releases/download/v0.9.0/SHA256SUMS");
        let anonymous = PublicGitHub::new(None).unwrap();
        for url in [&metadata, &download] {
            let request = anonymous.request(url).unwrap().build().unwrap();
            assert!(request.headers().get(AUTHORIZATION).is_none(), "{url}");
            assert_eq!(
                request.headers()["Accept"],
                "application/vnd.github+json",
                "{url}"
            );
        }

        let authenticated = PublicGitHub::new(Some("ghs_fake")).unwrap();
        let request = authenticated.request(&metadata).unwrap().build().unwrap();
        let header = &request.headers()[AUTHORIZATION];
        assert_eq!(header, "Bearer ghs_fake");
        assert!(header.is_sensitive());
        // Listing pages are built here, never taken from a Link header, and
        // carry the metadata token like the first listing request.
        let page = authenticated
            .checked_request(listing_page_url(2).unwrap())
            .build()
            .unwrap();
        assert_eq!(
            page.url().as_str(),
            "https://api.github.com/repos/replygirl/kuru/releases?page=2"
        );
        assert_eq!(page.headers()[AUTHORIZATION], "Bearer ghs_fake");
        for url in [
            download.as_str(),
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://api.github.com:8443/repos/replygirl/kuru/releases",
            "https://api.github.com.example/repos/replygirl/kuru/releases",
        ] {
            let request = authenticated.request(url).unwrap().build().unwrap();
            assert!(request.headers().get(AUTHORIZATION).is_none(), "{url}");
        }

        for url in [
            "http://api.github.com/repos/replygirl/kuru/releases",
            "https://user:pass@api.github.com/repos/replygirl/kuru/releases",
            "https://api.github.com/repos/replygirl/kuru/releases?per_page=1",
            "https://api.github.com/repos/replygirl/kuru/releases#top",
        ] {
            assert!(authenticated.request(url).is_err(), "{url}");
        }
        assert!(PublicGitHub::new(Some("bad\ntoken")).is_err());
    }

    mod download_timeouts {
        use super::*;
        use crate::archive::paced_http::{Pace, PacedServer};
        use std::time::Duration;

        const IDLE: Duration = Duration::from_millis(500);
        const GAP: Duration = Duration::from_millis(50);
        const CHUNKS: usize = 30;
        const OLD_TOTAL: Duration = Duration::from_millis(400);
        const BOUND: Duration = Duration::from_secs(5);

        /// The production shape with small values, without `https_only` so
        /// the local HTTP fixture is reachable.
        fn github(idle: Duration) -> PublicGitHub {
            PublicGitHub::with_client(
                Client::builder()
                    .no_proxy()
                    .redirect(Policy::limited(5))
                    .connect_timeout(Duration::from_secs(1))
                    .read_timeout(idle)
                    .user_agent("kuru-published-release-client")
                    .build()
                    .unwrap(),
                None,
            )
        }

        /// The same send and body phases as [`PublicGitHub::get`], through
        /// the scheme-agnostic request builder.
        async fn fetch(github: &PublicGitHub, url: Url) -> Result<Vec<u8>> {
            tokio::time::timeout(BOUND, async {
                bounded_body(send(github.checked_request(url)).await?, METADATA_LIMIT).await
            })
            .await
            .expect("published read must end within its idle bound")
        }

        #[tokio::test]
        async fn slow_response_past_the_old_total_succeeds_within_the_idle_bound() {
            let server = PacedServer::start(Pace::Trickle {
                chunks: CHUNKS,
                gap: GAP,
            })
            .await;
            let started = std::time::Instant::now();
            let bytes = fetch(&github(IDLE), server.url.clone()).await.unwrap();
            assert_eq!(bytes, PacedServer::body(CHUNKS));
            assert!(started.elapsed() > OLD_TOTAL, "{:?}", started.elapsed());
        }

        #[tokio::test]
        async fn a_body_stalled_past_idle_fails_naming_the_read_phase() {
            let server = PacedServer::start(Pace::StallBody).await;
            let error = fetch(&github(Duration::from_millis(200)), server.url.clone())
                .await
                .unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("read published release response body"),
                "{message}"
            );
            assert!(
                !message.contains("send published release request"),
                "{message}"
            );
        }

        #[tokio::test]
        async fn an_unanswered_connection_fails_naming_the_send_phase() {
            let server = PacedServer::never_accepting().await;
            let error = fetch(&github(Duration::from_millis(200)), server.url.clone())
                .await
                .unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("send published release request"),
                "{message}"
            );
        }

        #[tokio::test]
        async fn a_refused_connection_fails_naming_the_send_phase() {
            let url = PacedServer::refused().await;
            let error = fetch(&github(IDLE), url).await.unwrap_err();
            assert!(
                format!("{error:#}").contains("send published release request"),
                "{error:#}"
            );
        }
    }
}
