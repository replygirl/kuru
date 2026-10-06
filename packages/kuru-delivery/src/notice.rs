//! Personal, bounded release availability advice; never installation authority.

use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, ensure};
use kuru_platform::fs::{Directory, Publication};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use url::Url;

use crate::{archive, staging::Stage};

const ENDPOINT: &str = "https://github.com/replygirl/kuru/releases/latest/download/SHA256SUMS";
const CACHE_LIMIT: usize = 4096;
const MANIFEST_LIMIT: usize = 64 * 1024;
const INTERVAL: u64 = 24 * 60 * 60;
const FUTURE_SKEW: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Newer,
    Current,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Failure {
    Offline,
    Http,
    Malformed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    schema_version: u8,
    checked_at: u64,
    checked_by: String,
    outcome: Outcome,
    latest: Option<String>,
    failure: Option<Failure>,
}

fn numbers(version: &str) -> Result<[u64; 3]> {
    ensure!(version.len() <= 64, "version exceeds limit");
    let version = archive::checked_version(version)?;
    let triple = version
        .split_once('-')
        .map_or(version, |(triple, _)| triple);
    let mut parts = triple.split('.');
    let mut values = [0; 3];
    for value in &mut values {
        let part = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid version"))?;
        ensure!(part == "0" || !part.starts_with('0'), "invalid version");
        *value = part.parse()?;
    }
    Ok(values)
}

fn stable(version: &str) -> Result<[u64; 3]> {
    ensure!(
        !version.contains('-') && !version.starts_with('v'),
        "not a stable version"
    );
    numbers(version)
}

/// Select one exact host archive from the bounded public checksum manifest.
/// Neither its bytes nor archive names grant execution authority.
pub fn latest_version(manifest: &[u8], target: &str) -> Result<String> {
    ensure!(manifest.len() <= MANIFEST_LIMIT, "manifest exceeds limit");
    let target = crate::targets::find(target)?;
    let suffix = format!("-{}.{}", target.triple, target.format.extension());
    let text = std::str::from_utf8(manifest)?;
    let mut found = None;
    for line in text.lines() {
        let Some((hash, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some(name) = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*')) else {
            continue;
        };
        let Some(version) = name
            .strip_prefix("kuru-")
            .and_then(|name| name.strip_suffix(&suffix))
        else {
            continue;
        };
        ensure!(found.is_none(), "ambiguous host manifest");
        ensure!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid host checksum"
        );
        stable(version)?;
        found = Some(version.to_owned());
    }
    found.ok_or_else(|| anyhow::anyhow!("host archive missing"))
}

impl Cache {
    fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unknown notice cache");
        numbers(&self.checked_by)?;
        match self.outcome {
            Outcome::Failed => ensure!(
                self.latest.is_none() && self.failure.is_some(),
                "invalid failure cache"
            ),
            Outcome::Newer | Outcome::Current => {
                ensure!(self.failure.is_none(), "invalid success cache");
                let latest = stable(
                    self.latest
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("latest missing"))?,
                )?;
                ensure!(
                    (latest > numbers(&self.checked_by)?) == (self.outcome == Outcome::Newer),
                    "invalid version outcome"
                );
            }
        }
        Ok(())
    }

    fn due(&self, running: &str, now: u64) -> bool {
        self.checked_by != running
            || self.checked_at > now.saturating_add(FUTURE_SKEW)
            || now.saturating_sub(self.checked_at) >= INTERVAL
    }

    fn advice(&self, running: &str, hint: Option<&str>) -> Option<String> {
        let latest = self.latest.as_deref()?;
        if stable(latest).ok()? <= numbers(running).ok()? {
            return None;
        }
        let command = hint.map(str::to_owned).unwrap_or_else(|| format!(
            "kuru update --version {latest} --release-base https://github.com/replygirl/kuru/releases/download/v{latest}"
        ));
        Some(format!(
            "Kuru {latest} is available (installed {running}). Update: {command}"
        ))
    }
}

struct Stored {
    directory: Directory,
    previous: Option<File>,
    cache: Option<Cache>,
}

impl Stored {
    fn open(data: PathBuf) -> Result<Self> {
        let directory = Directory::ensure_private(&data.join("update"))?;
        let previous = match directory.read(OsStr::new("notice.json")) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let cache = if let Some(file) = &previous {
            let mut bytes = Vec::new();
            file.take(CACHE_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
            directory.verify(OsStr::new("notice.json"), file)?;
            if bytes.len() <= CACHE_LIMIT {
                serde_json::from_slice::<Cache>(&bytes)
                    .ok()
                    .filter(|cache| cache.validate().is_ok())
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            directory,
            previous,
            cache,
        })
    }

    fn save(&self, cache: &Cache) -> Result<()> {
        cache.validate()?;
        let bytes = serde_json::to_vec(cache)?;
        ensure!(bytes.len() <= CACHE_LIMIT, "cache exceeds limit");
        let stage = Stage::create(&self.directory, ".notice-")?;
        let mut payload = stage.directory().create_new(OsStr::new("notice.json"))?;
        payload.write_all(&bytes)?;
        payload.sync_all()?;
        let mode = match &self.previous {
            Some(file) => {
                self.directory.verify(OsStr::new("notice.json"), file)?;
                Publication::ReplaceRegular
            }
            None => Publication::New,
        };
        self.directory.publish_file(
            stage.directory(),
            OsStr::new("notice.json"),
            &payload,
            OsStr::new("notice.json"),
            mode,
        )?;
        drop(payload);
        stage.finish()?;
        Ok(())
    }
}

fn clock() -> Option<u64> {
    Some(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs())
}

fn endpoint() -> Result<Url> {
    #[cfg(feature = "test-support")]
    if let Ok(endpoint) = std::env::var("KURU_TEST_UPDATE_NOTICE_ENDPOINT") {
        let url = Url::parse(&endpoint)?;
        validate_url(&url)?;
        return Ok(url);
    }
    Ok(Url::parse(ENDPOINT)?)
}

#[cfg(feature = "test-support")]
fn validate_url(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid notice destination"
    );
    Ok(())
}

fn client() -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .user_agent("kuru-update-notice")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5
                || attempt.url().scheme() != "https"
                || attempt.url().host_str().is_none()
                || !attempt.url().username().is_empty()
                || attempt.url().password().is_some()
                || attempt.url().fragment().is_some()
            {
                attempt.error("invalid notice redirect")
            } else {
                attempt.follow()
            }
        }));
    #[cfg(feature = "test-support")]
    let builder = if let Some(path) = std::env::var_os("KURU_TEST_UPDATE_NOTICE_CA_PEM") {
        let mut pem = Vec::new();
        File::open(path)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut pem)?;
        ensure!(
            !pem.is_empty() && pem.len() <= 16 * 1024,
            "invalid fixture CA"
        );
        builder.tls_certs_merge([reqwest::Certificate::from_pem(&pem)?])
    } else {
        builder
    };
    Ok(builder.build()?)
}

async fn fetch(target: &str) -> std::result::Result<String, Failure> {
    let client = client().map_err(|_| Failure::Offline)?;
    let url = endpoint().map_err(|_| Failure::Malformed)?;
    let mut response = client.get(url).send().await.map_err(|_| Failure::Offline)?;
    if !response.status().is_success() {
        return Err(Failure::Http);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MANIFEST_LIMIT as u64)
    {
        return Err(Failure::Malformed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Offline)? {
        if bytes.len().saturating_add(chunk.len()) > MANIFEST_LIMIT {
            return Err(Failure::Malformed);
        }
        bytes.extend_from_slice(&chunk);
    }
    latest_version(&bytes, target).map_err(|_| Failure::Malformed)
}

fn publish(result: &Mutex<Option<String>>, advice: Option<String>) {
    if let Ok(mut slot) = result.try_lock() {
        *slot = advice;
    }
}

/// One advisory task; no provider, installation, session or memory authority.
/// Taking the result aborts without waiting on network or a task join.
pub struct Session {
    result: Arc<Mutex<Option<String>>>,
    task: JoinHandle<()>,
}

impl Session {
    pub fn start(data: PathBuf, running: String, target: String) -> Self {
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        let task = tokio::spawn(async move {
            #[cfg(unix)]
            let manager_hint = std::env::current_exe()
                .ok()
                .and_then(|path| {
                    crate::ownership::resolved_manager(
                        &path,
                        &crate::ownership::OwnershipEnv::capture(),
                    )
                    .ok()
                    .flatten()
                })
                .map(crate::ownership::Manager::update_hint);
            #[cfg(not(unix))]
            let manager_hint = None;
            let Some(now) = clock() else {
                return;
            };
            if numbers(&running).is_err() {
                return;
            }
            let Ok(stored) = Stored::open(data) else {
                return;
            };
            if let Some(cache) = &stored.cache {
                publish(&output, cache.advice(&running, manager_hint));
                if !cache.due(&running, now) {
                    return;
                }
            }
            let (outcome, latest, failure) = match fetch(&target).await {
                Ok(latest) => {
                    let Ok(current) = numbers(&running) else {
                        return;
                    };
                    let Ok(available) = stable(&latest) else {
                        return;
                    };
                    (
                        if available > current {
                            Outcome::Newer
                        } else {
                            Outcome::Current
                        },
                        Some(latest),
                        None,
                    )
                }
                Err(failure) => (Outcome::Failed, None, Some(failure)),
            };
            let cache = Cache {
                schema_version: 1,
                checked_at: now,
                checked_by: running.clone(),
                outcome,
                latest,
                failure,
            };
            let _ = stored.save(&cache);
            if outcome != Outcome::Failed {
                publish(&output, cache.advice(&running, manager_hint));
            }
        });
        Self { result, task }
    }

    pub fn finish(self) -> Option<String> {
        self.task.abort();
        self.result.try_lock().ok()?.take()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "aarch64-apple-darwin";
    fn manifest(version: &str) -> Vec<u8> {
        format!("{}  kuru-{version}-{TARGET}.tar.gz\n", "a".repeat(64)).into_bytes()
    }
    fn cache(now: u64) -> Cache {
        Cache {
            schema_version: 1,
            checked_at: now,
            checked_by: "0.9.0".into(),
            outcome: Outcome::Newer,
            latest: Some("0.10.0".into()),
            failure: None,
        }
    }

    #[test]
    fn notice_manifest_selects_exact_stable_host_and_rejects_ambiguous_bytes() {
        assert_eq!(
            latest_version(&manifest("0.10.0"), TARGET).unwrap(),
            "0.10.0"
        );
        let mut multiple = manifest("0.10.0");
        multiple.extend_from_slice(&manifest("0.11.0"));
        for bytes in [
            multiple,
            manifest("0.10.0-rc.1"),
            manifest("0.01.0"),
            manifest("18446744073709551616.0.0"),
            vec![b'x'; MANIFEST_LIMIT + 1],
        ] {
            assert!(latest_version(&bytes, TARGET).is_err());
        }
        assert!(latest_version(&manifest("0.10.0"), "x86_64-pc-windows-msvc").is_err());
        let mut bad = manifest("0.10.0");
        bad[0] = b'Z';
        assert!(latest_version(&bad, TARGET).is_err());
    }

    #[test]
    fn notice_cache_due_and_advice_do_not_promote_invalid_or_failed_records() {
        let now = 200000;
        let mut record = cache(now);
        record.validate().unwrap();
        assert!(!record.due("0.9.0", now + INTERVAL - 1));
        assert!(record.due("0.9.0", now + INTERVAL));
        assert!(record.due("0.9.1", now));
        record.checked_at = now + FUTURE_SKEW + 1;
        assert!(record.due("0.9.0", now));
        assert!(
            record
                .advice("0.9.0", Some("brew upgrade kuru"))
                .unwrap()
                .ends_with("Update: brew upgrade kuru")
        );
        assert!(record.advice("0.11.0", None).is_none());
        record.latest = Some("0.10.0\u{1b}[2J".into());
        assert!(record.validate().is_err());
        record.latest = None;
        record.outcome = Outcome::Failed;
        record.failure = Some(Failure::Offline);
        record.checked_at = now;
        record.validate().unwrap();
        assert!(!record.due("0.9.0", now + 1));
        assert!(record.advice("0.9.0", None).is_none());
        record.failure = None;
        assert!(record.validate().is_err());
    }

    #[test]
    fn notice_cache_checked_publication_retains_changed_target() {
        let root = tempfile::tempdir().unwrap();
        let record = cache(200000);
        let stored = Stored::open(root.path().to_owned()).unwrap();
        stored.save(&record).unwrap();
        let observed = Stored::open(root.path().to_owned()).unwrap();
        assert_eq!(
            observed.cache.as_ref().unwrap().latest.as_deref(),
            Some("0.10.0")
        );
        let stage = Stage::create(&observed.directory, ".replacement-").unwrap();
        let mut different = stage
            .directory()
            .create_new(OsStr::new("notice.json"))
            .unwrap();
        different.write_all(b"foreign-cache-bytes").unwrap();
        different.sync_all().unwrap();
        observed
            .directory
            .publish_file(
                stage.directory(),
                OsStr::new("notice.json"),
                &different,
                OsStr::new("notice.json"),
                Publication::ReplaceRegular,
            )
            .unwrap();
        drop(different);
        stage.finish().unwrap();
        assert!(observed.save(&record).is_err());
        assert_eq!(
            std::fs::read(root.path().join("update/notice.json")).unwrap(),
            b"foreign-cache-bytes"
        );
        assert!(
            std::fs::read_dir(root.path().join("update"))
                .unwrap()
                .all(|entry| entry.unwrap().file_name() == "notice.json")
        );
    }

    #[cfg(unix)]
    #[test]
    fn notice_cache_refuses_linked_public_or_symlinked_inputs() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let stored = Stored::open(root.path().to_owned()).unwrap();
        stored.save(&cache(200000)).unwrap();
        let path = root.path().join("update/notice.json");
        std::fs::hard_link(&path, root.path().join("other-link")).unwrap();
        assert!(Stored::open(root.path().to_owned()).is_err());
        std::fs::remove_file(root.path().join("other-link")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Stored::open(root.path().to_owned()).is_err());
        std::fs::remove_file(&path).unwrap();
        symlink(root.path().join("outside"), &path).unwrap();
        assert!(Stored::open(root.path().to_owned()).is_err());
    }

    #[tokio::test]
    async fn notice_cached_newer_finishes_without_network_or_raw_cache_output() {
        let root = tempfile::tempdir().unwrap();
        Stored::open(root.path().to_owned())
            .unwrap()
            .save(&cache(clock().unwrap()))
            .unwrap();
        let mut session = Session::start(root.path().to_owned(), "0.9.0".into(), TARGET.into());
        tokio::time::timeout(Duration::from_secs(5), &mut session.task)
            .await
            .unwrap()
            .unwrap();
        let text = session.finish().unwrap();
        assert!(text.starts_with("Kuru 0.10.0 is available (installed 0.9.0)."));
        assert!(!text.contains("checked_at"));
        assert!(
            std::fs::read_dir(root.path().join("update"))
                .unwrap()
                .all(|entry| entry.unwrap().file_name() == "notice.json")
        );
    }
}
