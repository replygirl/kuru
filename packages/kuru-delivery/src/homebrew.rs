//! Binary Homebrew distribution derived from one verified native release.

use crate::{
    archive,
    release::{GitHub, Version},
    shell_support, targets,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const TAP_REPOSITORY: &str = "replygirl/homebrew-kuru";
pub const SOURCE_REPOSITORY: &str = "replygirl/kuru";
pub const FORMULA_PATH: &str = "Formula/kuru.rb";
pub const MAX_FORMULA_BYTES: usize = 64 * 1024;
const TEMPLATE: &str = include_str!("../support/homebrew/kuru.rb.in");

/// Emit only immutable GitHub URLs. The caller authenticates the complete
/// candidate/public release before passing its asset hashes to this function.
pub fn generate(
    version: Version,
    repository: &str,
    hashes: &BTreeMap<String, String>,
) -> Result<String> {
    ensure!(
        repository == SOURCE_REPOSITORY,
        "Homebrew assets must come from {SOURCE_REPOSITORY}"
    );
    let version = version.to_string();
    let mut expected = BTreeSet::from(["SHA256SUMS".to_owned()]);
    for target in targets::CATALOG {
        expected.insert(archive::archive_name(&version, target.triple)?);
        expected.insert(shell_support::archive_name(&version, target.triple)?);
    }
    ensure!(
        hashes.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "Homebrew requires one complete verified release asset inventory"
    );
    ensure!(
        hashes.values().all(|hash| hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))),
        "Homebrew asset checksum must be lowercase SHA-256"
    );
    let mut formula = TEMPLATE
        .replace("@VERSION@", &version)
        .replace("@REPOSITORY@", repository);
    for (key, target) in [
        ("MAC_ARM", "aarch64-apple-darwin"),
        ("LINUX_ARM", "aarch64-unknown-linux-gnu"),
        ("LINUX_INTEL", "x86_64-unknown-linux-gnu"),
    ] {
        for (suffix, name) in [
            ("CORE", archive::archive_name(&version, target)?),
            ("SUPPORT", shell_support::archive_name(&version, target)?),
        ] {
            formula = formula.replace(&format!("@{key}_{suffix}@"), &hashes[&name]);
        }
    }
    ensure!(
        !formula.contains('@') && formula.len() <= MAX_FORMULA_BYTES,
        "Homebrew formula template is incomplete or oversized"
    );
    Ok(formula)
}

fn formula_version(formula: &str) -> Result<Version> {
    ensure!(
        !formula.is_empty() && formula.len() <= MAX_FORMULA_BYTES,
        "Homebrew formula must be bounded and nonempty"
    );
    let versions = formula
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("version \"")
                .and_then(|value| value.strip_suffix('"'))
        })
        .collect::<Vec<_>>();
    ensure!(
        versions.len() == 1,
        "Homebrew formula must declare exactly one version"
    );
    versions[0].parse()
}

/// Compare-and-swap the initialized dedicated tap's main branch. Identical
/// retries perform no write. `true` means a new formula commit was published.
pub async fn publish(api: &GitHub, version: Version, formula: &str) -> Result<bool> {
    ensure!(
        api.repository == TAP_REPOSITORY,
        "Homebrew publication requires the dedicated {TAP_REPOSITORY} repository"
    );
    ensure!(
        formula_version(formula)? == version,
        "Homebrew formula version differs from selected release"
    );
    let path = format!("repos/{}/contents/{FORMULA_PATH}", api.repository);
    let current = api
        .api(Method::GET, &format!("{path}?ref=main"), None, true)
        .await?;
    let mut payload = json!({"message":format!("chore: update kuru to v{version}"), "content":STANDARD.encode(formula), "branch":"main"});
    if let Some(current) = current {
        ensure!(
            current["type"] == "file"
                && current["path"] == FORMULA_PATH
                && current["encoding"] == "base64",
            "tap formula is not the expected regular Base64 file"
        );
        let encoded = current["content"]
            .as_str()
            .context("tap formula content is absent")?;
        ensure!(
            encoded.len() <= MAX_FORMULA_BYTES * 2,
            "tap formula exceeds size limit"
        );
        let encoded = encoded
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect::<String>();
        let bytes = STANDARD
            .decode(encoded)
            .context("tap formula has invalid Base64 content")?;
        ensure!(
            current["size"].as_u64() == Some(bytes.len() as u64),
            "tap formula size disagrees with content"
        );
        let previous = std::str::from_utf8(&bytes).context("tap formula is not UTF-8")?;
        let previous_version = formula_version(previous)?;
        ensure!(
            previous_version <= version,
            "refusing Homebrew version rollback from {previous_version} to {version}"
        );
        if previous == formula {
            return Ok(false);
        }
        ensure!(
            previous_version < version,
            "tap has conflicting formula bytes for v{version}"
        );
        let sha = current["sha"]
            .as_str()
            .context("tap formula blob SHA is absent")?;
        crate::release::checked_sha(sha)?;
        payload["sha"] = Value::String(sha.to_owned());
    }
    let response = api
        .api(Method::PUT, &path, Some(payload), false)
        .await?
        .context("tap publication returned no result")?;
    ensure!(
        response["content"]["path"] == FORMULA_PATH
            && response["content"]["sha"]
                .as_str()
                .is_some_and(|sha| crate::release::checked_sha(sha).is_ok())
            && response["commit"]["sha"]
                .as_str()
                .is_some_and(|sha| crate::release::checked_sha(sha).is_ok()),
        "tap publication returned an incomplete commit receipt; inspect the tap before retrying"
    );
    Ok(true)
}
