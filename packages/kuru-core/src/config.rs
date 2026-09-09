use std::{
    collections::BTreeMap,
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Framework, Mode};

const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_COMBINED_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
}

/// Interactive choices belong to the canonical project, independent of chats
/// and framework memory. Absence of a provider entry means no remembered choice;
/// an entry whose effort is None explicitly requests the provider's default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectPreferences {
    pub mode: Option<Mode>,
    pub providers: BTreeMap<String, ModelPreference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelPreference {
    pub model: String,
    pub effort: Option<String>,
}

/// Explicit invocation selections are applied before semantic validation, so a
/// temporary framework override is checked against that framework's part count.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectionOverrides<'a> {
    pub mode: Option<Mode>,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
}

impl ProjectPreferences {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.providers.len() <= 64,
            "too many saved provider choices"
        );
        for (provider, choice) in &self.providers {
            alias("saved provider", provider)?;
            nonempty("saved model", &choice.model, 256)?;
            if let Some(effort) = &choice.effort {
                nonempty("saved effort", effort, 128)?;
            }
        }
        Ok(())
    }

    fn overlay(
        &self,
        merged: &mut toml::Value,
        provider: &str,
        explicit_model: Option<&str>,
    ) -> Result<()> {
        self.validate()?;
        if let Some(mode) = self.mode {
            merged["mode"] = toml::Value::try_from(mode)?;
        }
        if let Some(choice) = self.providers.get(provider)
            && explicit_model.is_none_or(|model| model == choice.model)
        {
            merged["model"] = choice.model.clone().into();
            let table = merged
                .as_table_mut()
                .context("configuration is not a table")?;
            if let Some(effort) = &choice.effort {
                table.insert("effort".into(), effort.clone().into());
            } else {
                table.remove("effort");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub provider: String,
    pub model: String,
    pub effort: Option<String>,
    pub max_rounds: usize,
    pub max_tool_calls: usize,
    pub max_parallel: usize,
    /// Zero disables periodic dreaming; explicit and exit dreaming remain available.
    pub dream_every: usize,
    pub dream_on_exit: bool,
    pub max_parts: usize,
    pub allow_shell: bool,
    pub allow_write: bool,
    pub codex_command: String,
    pub api_base: String,
    pub api_key_env: String,
    pub mcp: BTreeMap<String, McpConfig>,
    pub external_agents: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Ifs,
            provider: "codex".into(),
            model: "auto".into(),
            effort: None,
            max_rounds: 3,
            max_tool_calls: 12,
            max_parallel: 4,
            dream_every: 8,
            dream_on_exit: true,
            max_parts: 16,
            allow_shell: false,
            allow_write: false,
            codex_command: "codex".into(),
            api_base: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            mcp: BTreeMap::new(),
            external_agents: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Merge user, outer-to-inner project, then explicit local TOML.
    ///
    /// Missing ancestor files are normal. An explicitly provided user or local
    /// path must exist. Maps merge recursively and arrays replace. Each layer is
    /// checked for unknown keys and field types; semantic checks run after merging.
    pub fn load(user: Option<&Path>, project: &Path, local: Option<&Path>) -> Result<Self> {
        Self::load_with_preferences(
            user,
            project,
            local,
            &ProjectPreferences::default(),
            SelectionOverrides::default(),
        )
    }

    /// Remembered interactive choices override ordinary file defaults. An
    /// explicit local file and CLI model/provider selection remain authoritative.
    /// When an invocation changes model, it does not inherit another model's
    /// remembered effort; its own file settings or provider default apply.
    pub fn load_with_preferences(
        user: Option<&Path>,
        project: &Path,
        local: Option<&Path>,
        preferences: &ProjectPreferences,
        overrides: SelectionOverrides<'_>,
    ) -> Result<Self> {
        let ancestors = ancestor_directories(project)?;
        let mut layers = Vec::new();
        if let Some(path) = user {
            layers.push((path.to_path_buf(), true));
        }
        layers.extend(
            ancestors
                .into_iter()
                .map(|path| (path.join(".kuru/config.toml"), false)),
        );
        let mut merged = toml::Value::try_from(Self::default())?;
        let mut total_bytes = 0;
        for (path, required) in layers {
            let Some(source) = read_bounded(&path, required)? else {
                continue;
            };
            total_bytes += source.len();
            ensure!(
                total_bytes <= MAX_COMBINED_BYTES,
                "combined configuration exceeds 1 MiB"
            );
            let patch: toml::Value = toml::from_str(&source)
                .with_context(|| format!("cannot parse configuration {}", path.display()))?;
            merge(&mut merged, patch);
            // Deserialize at every step so an invalid layer cannot be silently
            // masked by a later override. Defaults live in one place, Config.
            let _: Self = merged
                .clone()
                .try_into()
                .with_context(|| format!("invalid configuration in {}", path.display()))?;
        }
        let local_patch = if let Some(path) = local {
            let source = read_bounded(path, true)?.context("explicit configuration is missing")?;
            total_bytes += source.len();
            ensure!(
                total_bytes <= MAX_COMBINED_BYTES,
                "combined configuration exceeds 1 MiB"
            );
            Some(
                toml::from_str::<toml::Value>(&source)
                    .with_context(|| format!("cannot parse configuration {}", path.display()))?,
            )
        } else {
            None
        };
        let provider = overrides
            .provider
            .or_else(|| local_patch.as_ref()?.get("provider")?.as_str())
            .or_else(|| merged.get("provider")?.as_str())
            .context("provider must be a string")?
            .to_owned();
        let explicit_model = overrides
            .model
            .or_else(|| local_patch.as_ref()?.get("model")?.as_str());
        preferences.overlay(&mut merged, &provider, explicit_model)?;
        if let Some(patch) = local_patch {
            merge(&mut merged, patch);
        }
        let mut config: Self = merged.try_into().with_context(|| {
            local.map_or_else(
                || "invalid effective configuration".into(),
                |path| format!("invalid configuration in {}", path.display()),
            )
        })?;
        if let Some(mode) = overrides.mode {
            config.mode = mode;
        }
        if let Some(provider) = overrides.provider {
            config.provider = provider.into();
        }
        if let Some(model) = overrides.model {
            config.model = model.into();
        }
        if let Some(effort) = overrides.effort {
            config.effort = Some(effort.into());
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.provider.as_str(), "codex" | "responses" | "demo"),
            "provider must be codex, responses, or demo"
        );
        nonempty("model", &self.model, 256)?;
        if let Some(effort) = &self.effort {
            nonempty("effort", effort, 128)?;
        }
        bounded("max_rounds", self.max_rounds, 1, 64)?;
        bounded("max_tool_calls", self.max_tool_calls, 1, 1024)?;
        bounded("max_parallel", self.max_parallel, 1, 64)?;
        bounded(
            "max_parts",
            self.max_parts,
            Framework::builtin(self.mode).parts.len(),
            128,
        )?;
        nonempty("codex_command", &self.codex_command, 4096)?;
        endpoint("api_base", &self.api_base)?;
        environment_name("api_key_env", &self.api_key_env)?;
        ensure!(
            self.mcp.len() <= 64,
            "at most 64 MCP servers may be configured"
        );
        ensure!(
            self.external_agents.len() <= 64,
            "at most 64 external agents may be configured"
        );
        for (name, mcp) in &self.mcp {
            alias("MCP server", name)?;
            match (&mcp.command, &mcp.url) {
                (Some(command), None) => nonempty("MCP command", command, 4096)?,
                (None, Some(url)) => {
                    endpoint("MCP url", url)?;
                    ensure!(
                        mcp.args.is_empty() && mcp.env.is_empty(),
                        "HTTP MCP servers cannot specify process args or env"
                    );
                }
                _ => bail!("MCP server {name} must specify exactly one of command or url"),
            }
            ensure!(mcp.args.len() <= 256, "MCP process has too many arguments");
            ensure!(
                mcp.args.iter().all(|arg| !arg.contains('\0')),
                "MCP arguments cannot contain NUL characters"
            );
            for (key, value) in &mcp.env {
                environment_name("MCP environment key", key)?;
                ensure!(
                    !value.contains('\0'),
                    "MCP environment values cannot contain NUL characters"
                );
            }
        }
        for (name, url) in &self.external_agents {
            alias("external agent", name)?;
            endpoint("external agent URL", url)?;
        }
        Ok(())
    }
}

fn bounded(name: &str, value: usize, min: usize, max: usize) -> Result<()> {
    ensure!(
        (min..=max).contains(&value),
        "{name} must be between {min} and {max}"
    );
    Ok(())
}

fn nonempty(name: &str, value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max && !value.contains('\0'),
        "{name} must be nonempty, at most {max} bytes, and contain no NUL characters"
    );
    Ok(())
}

fn alias(kind: &str, value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
        "{kind} names must use 1–64 ASCII letters, digits, underscores, or hyphens"
    );
    Ok(())
}

fn environment_name(label: &str, value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 256
            && value.bytes().enumerate().all(|(i, byte)| byte == b'_'
                || byte.is_ascii_alphabetic()
                || (i > 0 && byte.is_ascii_digit())),
        "{label} must be a portable environment variable name"
    );
    Ok(())
}

fn endpoint(label: &str, value: &str) -> Result<()> {
    // Avoid putting URL values in errors: a malformed URL might contain a secret.
    let url =
        Url::parse(value).with_context(|| format!("{label} must be an absolute HTTP(S) URL"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "{label} must be an absolute HTTP(S) URL"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "{label} must not contain embedded credentials"
    );
    ensure!(
        url.fragment().is_none(),
        "{label} must not contain a fragment"
    );
    Ok(())
}

fn merge(base: &mut toml::Value, patch: toml::Value) {
    match (base, patch) {
        (toml::Value::Table(base), toml::Value::Table(patch)) => {
            for (key, value) in patch {
                match base.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

fn ancestor_directories(project: &Path) -> Result<Vec<PathBuf>> {
    let project = project
        .canonicalize()
        .with_context(|| format!("cannot resolve project {}", project.display()))?;
    ensure!(project.is_dir(), "project path must be a directory");
    let mut ancestors: Vec<_> = project.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    Ok(ancestors)
}

fn read_bounded(path: &Path, required: bool) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound && !required => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    ensure!(
        file.metadata()?.is_file(),
        "{} must be a regular file",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    ensure!(
        bytes.len() <= MAX_FILE_BYTES,
        "{} exceeds the 256 KiB file limit",
        path.display()
    );
    String::from_utf8(bytes)
        .map(Some)
        .with_context(|| format!("{} is not UTF-8", path.display()))
}

/// Include every ancestor AGENTS.md, clearly identifying increasingly local scope.
/// Source order conveys precedence without attempting to reinterpret instructions.
pub fn load_instructions(project: &Path) -> Result<String> {
    let mut combined = String::new();
    let mut total_bytes = 0;
    for directory in ancestor_directories(project)? {
        let path = directory.join("AGENTS.md");
        let Some(source) = read_bounded(&path, false)? else {
            continue;
        };
        total_bytes += source.len();
        ensure!(
            total_bytes <= MAX_COMBINED_BYTES,
            "combined AGENTS.md instructions exceed 1 MiB"
        );
        if combined.is_empty() {
            combined.push_str("Project instructions follow from outermost to most local. Where instructions conflict, the most local applicable AGENTS.md takes precedence; higher-priority conversation instructions still apply.\n");
        }
        combined.push_str(&format!("\n--- AGENTS.md: {} ---\n", path.display()));
        combined.push_str(&source);
        combined.push('\n');
    }
    Ok(combined)
}
