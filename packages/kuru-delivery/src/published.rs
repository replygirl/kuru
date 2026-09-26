//! Public GitHub release metadata and the authenticated previous release.

use crate::{archive, release};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Client, RequestBuilder, Url,
    header::{AUTHORIZATION, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use std::{collections::HashSet, ffi::OsString, time::Duration};

pub(crate) const REPOSITORY: &str = "replygirl/kuru";
pub(crate) const METADATA_LIMIT: usize = 4 * 1024 * 1024;
const METADATA_HOST: &str = "api.github.com";

#[derive(Debug, Deserialize)]
pub(crate) struct ReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) digest: Option<String>,
}

#[derive(Debug, Deserialize)]
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
        Ok(Self {
            client: Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(Policy::limited(5))
                .timeout(Duration::from_secs(60))
                .user_agent("kuru-published-windows-verifier")
                .build()?,
            authorization,
        })
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
        let metadata = url.host_str() == Some(METADATA_HOST) && url.port().is_none();
        let mut request = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(authorization) = self.authorization.as_ref().filter(|_| metadata) {
            request = request.header(AUTHORIZATION, authorization.clone());
        }
        Ok(request)
    }

    pub(crate) async fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let mut response = self.request(url)?.send().await?.error_for_status()?;
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

/// The published stable release immediately preceding a candidate, with its
/// checksum manifest and one target's core archive already authenticated.
/// Its own updater is what existing installations of that target run against
/// the candidate. Acceptance from it is a floor, not the compatibility policy:
/// updating from any installed release must remain possible.
pub struct PreviousRelease {
    pub version: String,
    pub target: &'static str,
    pub manifest: Vec<u8>,
    pub archive: Vec<u8>,
    /// The paired shell-support envelope, present exactly when the previous
    /// release publishes one for this target.
    pub support: Option<Vec<u8>>,
}

/// The core archive and shell-support envelope names a release publishes for
/// one target: `.tar.gz` on Unix targets and `.zip` on Windows.
fn target_assets(version: &str, target: &str) -> Result<(String, String)> {
    Ok((
        archive::archive_name(version, target)?,
        crate::shell_support::archive_name(version, target)?,
    ))
}

/// Resolve and download the release preceding `candidate` for `target` from
/// public GitHub.
///
/// Nothing is pinned: the release is selected at run time by
/// [`select_previous`]. `token` authenticates only the release listing; every
/// asset is read anonymously over HTTPS from its immutable version path, and
/// no bytes are returned until the archive and any paired shell-support
/// envelope match that release's own `SHA256SUMS` and every file matches
/// GitHub's asset digest.
pub async fn previous_release(
    candidate: &str,
    target: &str,
    token: Option<&str>,
) -> Result<PreviousRelease> {
    let candidate = candidate.parse::<release::Version>()?;
    let target = crate::targets::find(target)?.triple;
    let github = PublicGitHub::new(token)?;
    // The unpaginated listing holds GitHub's 30 most recently created
    // releases, which always include the latest ones.
    let releases: Vec<PublishedRelease> = github.json("releases").await?;
    let version = select_previous(&releases, candidate)?.to_string();
    let listed = releases
        .iter()
        .find(|release| release.tag_name == format!("v{version}"))
        .context("selected previous release is absent from its listing")?;
    let (archive_name, support_name) = target_assets(&version, target)?;
    let manifest_digest = listed_digest(listed, &version, "SHA256SUMS")?;
    let archive_digest = listed_digest(listed, &version, &archive_name)?;
    let download_base = format!("https://github.com/{REPOSITORY}/releases/download/v{version}");
    let manifest = github
        .get(&format!("{download_base}/SHA256SUMS"), 64 * 1024)
        .await?;
    verify_manifest(&manifest, &manifest_digest)?;
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
    Ok(PreviousRelease {
        version,
        target,
        manifest,
        archive,
        support,
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
            let extension = if target.triple == WINDOWS_TARGET {
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
            listed("v0.9.0", false, false),
            listed("v0.8.0", false, false),
        ];
        assert_eq!(
            select_previous(&releases, version("0.9.1")).unwrap(),
            version("0.9.0")
        );
        // A branch older than the latest publication fails closed; rebase it.
        let ahead = [listed("v0.10.0", false, false)];
        assert!(select_previous(&ahead, version("0.9.1")).is_err());
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
}
