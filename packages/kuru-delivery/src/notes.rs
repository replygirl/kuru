//! Generate bounded release notes with the pinned external Communiqué tool.
use crate::release::{Version, checked_sha, git, rooted_command, workspace_version};
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path, time::Duration};

const MAX_CONTEXT_BYTES: usize = 100_000;
const PRODUCT_DOCS: [&str; 6] = [
    "apps/kuru-docs/concepts/frameworks.md",
    "apps/kuru-docs/concepts/memory.md",
    "apps/kuru-docs/concepts/sessions.md",
    "apps/kuru-docs/guide/authentication.md",
    "apps/kuru-docs/reference/configuration.md",
    "apps/kuru-docs/guide/first-conversation.md",
];

fn append_context(context: &mut String, text: &str) -> Result<()> {
    ensure!(
        context.len().saturating_add(text.len()) <= MAX_CONTEXT_BYTES,
        "release notes context exceeds limit"
    );
    context.push_str(text);
    Ok(())
}

pub async fn configuration(
    root: &Path,
    candidate: &str,
    selected: Version,
) -> Result<(String, Option<String>)> {
    checked_sha(candidate)?;
    ensure!(
        workspace_version(root, Some(candidate)).await? == selected,
        "notes commit does not contain selected version"
    );
    let mut config: toml::Value =
        toml::from_str(&git(root, &["show", &format!("{candidate}:communique.toml")]).await?)?;
    let table = config
        .as_table_mut()
        .context("Communiqué configuration must be a table")?;
    ensure!(
        table
            .keys()
            .all(|key| ["context", "system_extra", "defaults"].contains(&key.as_str())),
        "unsupported Communiqué configuration sections"
    );
    let tags = git(root, &["tag", "--merged", candidate, "--sort=-v:refname"]).await?;
    let previous = tags
        .lines()
        .find(|tag| {
            tag.parse::<Version>()
                .is_ok_and(|version| version < selected)
        })
        .map(str::to_owned);
    let mut context = table
        .get("context")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    append_context(
        &mut context,
        &format!("\n\nTarget release: v{selected}. Exact source commit: {candidate}."),
    )?;
    if previous.is_none() {
        append_context(
            &mut context,
            "\n\nInitial implementation inventory (the root commit is excluded from the tool's automatic log range):\n",
        )?;
        for commit in git(root, &["rev-list", "--max-parents=0", candidate])
            .await?
            .lines()
        {
            append_context(
                &mut context,
                &git(
                    root,
                    &[
                        "show",
                        "--root",
                        "--format=fuller",
                        "--stat",
                        "--no-renames",
                        checked_sha(commit)?,
                    ],
                )
                .await?,
            )?;
        }
    }
    append_context(
        &mut context,
        "\n\nCurrent product documentation (authoritative for present behavior at the exact source commit):\n",
    )?;
    for path in PRODUCT_DOCS {
        append_context(&mut context, &format!("\n--- {path} @ {candidate} ---\n"))?;
        append_context(
            &mut context,
            &git(root, &["show", &format!("{candidate}:{path}")]).await?,
        )?;
    }
    table.insert("context".into(), toml::Value::String(context));
    let defaults = table
        .get("defaults")
        .and_then(toml::Value::as_table)
        .context("missing Communiqué defaults")?;
    ensure!(
        defaults.iter().all(|(key, value)| key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && matches!(
                value,
                toml::Value::String(_) | toml::Value::Integer(_) | toml::Value::Boolean(_)
            )),
        "unsupported Communiqué default"
    );
    Ok((toml::to_string(&config)?, previous))
}

/// Explicit provider controls for isolated contract fixtures. Normal releases
/// inherit their configured API key and use the configured HTTPS URL.
#[derive(Default)]
pub struct ProviderOptions {
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub omit_github_context: bool,
}

pub async fn generate(
    root: &Path,
    candidate: &str,
    selected: Version,
    output: &Path,
    provider: &ProviderOptions,
) -> Result<()> {
    let (config, previous) = configuration(root, candidate, selected).await?;
    ensure!(
        git(root, &["rev-parse", "HEAD"]).await? == candidate,
        "notes checkout must match the exact release commit"
    );
    let tag = format!("v{selected}");
    if !git(root, &["tag", "--list", &tag]).await?.is_empty() {
        ensure!(
            git(root, &["rev-parse", &format!("{tag}^{{commit}}")]).await? == candidate,
            "existing notes tag points to a different commit"
        );
    }
    ensure!(
        fs::symlink_metadata(output).is_err(),
        "notes output already exists; choose an empty output path"
    );
    ensure!(
        git(root, &["status", "--porcelain", "--untracked-files=all"])
            .await?
            .is_empty(),
        "notes checkout has uncommitted changes; repository tools must read the exact release commit"
    );
    let temp = tempfile::tempdir()?;
    let config_path = temp.path().join("communique.toml");
    let notes_path = temp.path().join("notes.md");
    fs::write(&config_path, config)?;
    let mut command = rooted_command(root, "communique");
    command
        .args(["--config"])
        .arg(config_path)
        .args(["generate", &tag]);
    if let Some(previous) = previous {
        command.arg(previous);
    }
    command.arg("--output").arg(&notes_path).kill_on_drop(true);
    if let Some(endpoint) = &provider.endpoint {
        command.args(["--base-url", endpoint]);
    }
    if let Some(key) = &provider.api_key {
        command.env("OPENAI_API_KEY", key);
    }
    if provider.omit_github_context {
        command.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
    }
    let result = crate::command::output(&mut command, Duration::from_secs(600))
        .await
        .context("Communiqué execution failed; no release was published")?;
    ensure!(
        result.status.success(),
        "Communiqué failed ({}); no release was published",
        result.status
    );
    let metadata =
        fs::symlink_metadata(&notes_path).context("Communiqué did not produce release notes")?;
    ensure!(
        metadata.is_file() && metadata.len() <= 100_000,
        "notes must be a bounded regular file"
    );
    let text = fs::read_to_string(notes_path)?;
    ensure!(!text.trim().is_empty(), "Communiqué produced empty notes");
    let mut staged =
        tempfile::NamedTempFile::new_in(output.parent().unwrap_or_else(|| Path::new(".")))?;
    staged.write_all(text.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist_noclobber(output)?;
    Ok(())
}
