use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::permissions;
use crate::{
    Framework, Mode, PermissionAction, PermissionRule, PermissionSelector, ProjectRelativeTarget,
};

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

/// All invocation inputs captured before workspace authority is reviewed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvocationOverrides {
    pub mode: Option<Mode>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub allow_write: bool,
    pub allow_shell: bool,
    pub no_dream: bool,
}

impl From<SelectionOverrides<'_>> for InvocationOverrides {
    fn from(value: SelectionOverrides<'_>) -> Self {
        Self {
            mode: value.mode,
            provider: value.provider.map(str::to_owned),
            model: value.model.map(str::to_owned),
            effort: value.effort.map(str::to_owned),
            ..Self::default()
        }
    }
}

/// Authority effects whose effective value has an automatic ancestor origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityClaimCategory {
    WorkspaceWrite,
    Shell,
    McpStdio,
    McpHttp,
    MemoryDoltBinary,
    MemoryCacheDir,
    ResponsesRoute,
    ExternalAgent,
    ProjectInstructions,
    ToolPermissions,
}

impl AuthorityClaimCategory {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WorkspaceWrite => "workspace write",
            Self::Shell => "shell",
            Self::ToolPermissions => "tool permissions",
            Self::McpStdio => "stdio MCP",
            Self::McpHttp => "HTTP MCP",
            Self::MemoryDoltBinary => "memory executable",
            Self::MemoryCacheDir => "memory cache",
            Self::ResponsesRoute => "Responses route",
            Self::ExternalAgent => "external agent",
            Self::ProjectInstructions => "project instructions",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManifestDigest([u8; 32]);

impl ManifestDigest {
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Display for ManifestDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimDigest([u8; 32]);

impl ClaimDigest {
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Display for ClaimDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SafeSource(String);

impl SafeSource {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SafeSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeClaimDisplay(String);

impl SafeClaimDisplay {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SafeClaimDisplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The active Responses route after workspace preflight. Its values are not
/// redacted because only the approved caller may read the named environment.
pub struct ResponsesRouteConfig {
    api_base: String,
    api_key_env: String,
}

impl ResponsesRouteConfig {
    pub fn api_base(&self) -> &str {
        &self.api_base
    }
    pub fn api_key_env(&self) -> &str {
        &self.api_key_env
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityClaim {
    category: AuthorityClaimCategory,
    digest: ClaimDigest,
    source: SafeSource,
    sources: Vec<SafeSource>,
    source_digests: Vec<[u8; 32]>,
    display: SafeClaimDisplay,
}

impl AuthorityClaim {
    pub const fn category(&self) -> AuthorityClaimCategory {
        self.category
    }
    pub const fn digest(&self) -> ClaimDigest {
        self.digest
    }
    pub fn source(&self) -> &SafeSource {
        &self.source
    }
    pub fn sources(&self) -> &[SafeSource] {
        &self.sources
    }
    pub fn display(&self) -> &SafeClaimDisplay {
        &self.display
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityManifest {
    schema_version: u16,
    digest: ManifestDigest,
    claims: Vec<AuthorityClaim>,
    sources: Vec<SafeSource>,
}

impl AuthorityManifest {
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    pub const fn full_digest(&self) -> ManifestDigest {
        self.digest
    }
    pub fn claims(&self) -> &[AuthorityClaim] {
        &self.claims
    }
    pub fn sources(&self) -> &[SafeSource] {
        &self.sources
    }
    pub fn filtered(&self, categories: &BTreeSet<AuthorityClaimCategory>) -> SafeManifest {
        let claims: Vec<_> = self
            .claims
            .iter()
            .filter(|claim| categories.contains(&claim.category))
            .cloned()
            .collect();
        SafeManifest::new(self.schema_version, claims)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeManifest {
    schema_version: u16,
    digest: ManifestDigest,
    claims: Vec<AuthorityClaim>,
    sources: Vec<SafeSource>,
}

impl SafeManifest {
    fn new(schema_version: u16, claims: Vec<AuthorityClaim>) -> Self {
        let sources = claims
            .iter()
            .flat_map(|claim| claim.sources.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let digest = manifest_digest(schema_version, &claims);
        Self {
            schema_version,
            digest,
            claims,
            sources,
        }
    }
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    pub const fn digest(&self) -> ManifestDigest {
        self.digest
    }
    pub fn claims(&self) -> &[AuthorityClaim] {
        &self.claims
    }
    pub fn sources(&self) -> &[SafeSource] {
        &self.sources
    }
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
pub struct MemoryConfig {
    pub dolt_binary: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub offline: bool,
    pub startup_timeout_secs: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            dolt_binary: None,
            cache_dir: None,
            offline: false,
            startup_timeout_secs: 30,
        }
    }
}

impl MemoryConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=300).contains(&self.startup_timeout_secs),
            "memory.startup_timeout_secs must be between 1 and 300"
        );
        for (label, path) in [
            ("dolt_binary", &self.dolt_binary),
            ("cache_dir", &self.cache_dir),
        ] {
            if let Some(path) = path {
                ensure!(
                    !path.as_os_str().is_empty()
                        && !path.as_os_str().as_encoded_bytes().contains(&0),
                    "memory.{label} must be a nonempty path without NUL characters"
                );
                ensure!(
                    path.is_absolute(),
                    "memory.{label} must be an absolute path"
                );
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
    /// Used only when neither live discovery nor the offline catalog supplies a
    /// context window. It cannot override a verified provider limit.
    pub assumed_context_window_tokens: Option<u64>,
    pub max_rounds: usize,
    pub max_tool_calls: usize,
    pub max_parallel: usize,
    /// Zero disables periodic dreaming; explicit and exit dreaming remain available.
    pub dream_every: usize,
    pub dream_on_exit: bool,
    pub max_parts: usize,
    pub allow_shell: bool,
    pub allow_write: bool,
    pub permissions: Vec<PermissionRule>,
    pub api_base: String,
    pub api_key_env: String,
    pub mcp: BTreeMap<String, McpConfig>,
    pub external_agents: BTreeMap<String, String>,
    pub memory: MemoryConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Ifs,
            provider: "codex".into(),
            model: "auto".into(),
            effort: None,
            assumed_context_window_tokens: None,
            max_rounds: 3,
            max_tool_calls: 12,
            max_parallel: 4,
            dream_every: 8,
            dream_on_exit: true,
            max_parts: 16,
            allow_shell: false,
            allow_write: false,
            permissions: Vec::new(),
            api_base: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            mcp: BTreeMap::new(),
            external_agents: BTreeMap::new(),
            memory: MemoryConfig::default(),
        }
    }
}

/// A single parsed configuration view used for workspace review and activation.
/// Its fields are private so callers cannot add authority after review.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    workspace: PathBuf,
    merged: toml::Value,
    origins: BTreeMap<String, LayerOrigin>,
    local: Option<toml::Value>,
    local_origins: BTreeMap<String, LayerOrigin>,
    overrides: InvocationOverrides,
    manifest: AuthorityManifest,
    memory: MemoryConfig,
    instructions: String,
}

#[derive(Debug, Clone)]
struct LayerOrigin {
    source: SafeSource,
    source_digest: [u8; 32],
    automatic: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InstructionSourceKind {
    Agents,
}

impl InstructionSourceKind {
    const fn prompt_name(self) -> &'static str {
        match self {
            Self::Agents => "AGENTS.md",
        }
    }
}

#[derive(Debug, Clone)]
struct InstructionSource {
    kind: InstructionSourceKind,
    path: PathBuf,
    safe_source: SafeSource,
    path_digest: [u8; 32],
    identity: [u8; 24],
    content: String,
}

#[derive(Serialize)]
struct InstructionClaimEntry<'a> {
    kind: InstructionSourceKind,
    path_digest: &'a [u8; 32],
    identity: &'a [u8; 24],
    content: &'a str,
}

impl ConfigSnapshot {
    pub fn parse(
        user: Option<&Path>,
        workspace: &Path,
        local: Option<&Path>,
        overrides: InvocationOverrides,
    ) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| config_error("read", workspace))?;
        if !workspace.is_dir() {
            return Err(config_error("read", &workspace));
        }
        let instruction_sources = capture_instruction_sources(&workspace)?;
        let instructions = format_instructions(&instruction_sources);
        let mut merged =
            toml::Value::try_from(Config::default()).expect("default config serializes");
        let mut origins = BTreeMap::new();
        let mut total_bytes: usize = 0;
        let mut layers = Vec::new();
        if let Some(path) = user {
            layers.push((path.to_path_buf(), true, false));
        }
        for directory in
            ancestor_directories(&workspace).map_err(|_| config_error("read", &workspace))?
        {
            layers.push((directory.join(".kuru/config.toml"), false, true));
        }
        for (path, required, automatic) in layers {
            let Some(source) = read_config_bounded(&path, required)? else {
                continue;
            };
            total_bytes = total_bytes
                .checked_add(source.len())
                .ok_or_else(|| config_error("read", &path))?;
            if total_bytes > MAX_COMBINED_BYTES {
                return Err(config_error("read", &path));
            }
            let patch: toml::Value = toml::from_str(&source)
                .map_err(|error| config_parse_error(&path, &source, &error))?;
            reject_removed_settings(&patch).map_err(|_| config_error("validation", &path))?;
            merge_with_origins(
                &mut merged,
                &mut origins,
                patch,
                layer_origin(&path, automatic),
            );
            let _: Config = merged
                .clone()
                .try_into()
                .map_err(|_| config_error("type", &path))?;
        }
        let (local, local_origins) = if let Some(path) = local {
            let source =
                read_config_bounded(path, true)?.ok_or_else(|| config_error("read", path))?;
            total_bytes = total_bytes
                .checked_add(source.len())
                .ok_or_else(|| config_error("read", path))?;
            if total_bytes > MAX_COMBINED_BYTES {
                return Err(config_error("read", path));
            }
            let patch: toml::Value = toml::from_str(&source)
                .map_err(|error| config_parse_error(path, &source, &error))?;
            reject_removed_settings(&patch).map_err(|_| config_error("validation", path))?;
            let mut checked = merged.clone();
            merge(&mut checked, patch.clone());
            let _: Config = checked.try_into().map_err(|_| config_error("type", path))?;
            let mut local_origins = BTreeMap::new();
            record_origins(&patch, "", layer_origin(path, false), &mut local_origins);
            (Some(patch), local_origins)
        } else {
            (None, BTreeMap::new())
        };
        let provisional = Self {
            workspace,
            merged,
            origins,
            local,
            local_origins,
            overrides,
            manifest: empty_manifest(),
            memory: MemoryConfig::default(),
            instructions,
        };
        let (value, value_origins) =
            provisional.value_with_preferences(&ProjectPreferences::default())?;
        let config: Config = value
            .clone()
            .try_into()
            .map_err(|_| config_error("type", provisional.workspace()))?;
        config
            .memory
            .validate()
            .map_err(|_| config_error("validation", provisional.workspace()))?;
        permissions::validate_rules(&config.permissions)
            .map_err(|_| config_error("validation", provisional.workspace()))?;
        let manifest = derive_manifest(&config, &value_origins, &instruction_sources)?;
        Ok(Self {
            memory: config.memory.clone(),
            manifest,
            ..provisional
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn memory_config(&self) -> &MemoryConfig {
        &self.memory
    }
    pub fn manifest(&self) -> &AuthorityManifest {
        &self.manifest
    }
    /// Exact automatic project-instruction bytes captured before workspace review.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Return an active Responses route without loading saved preferences or memory.
    pub fn responses_route(&self) -> Result<Option<ResponsesRouteConfig>> {
        let (value, _) = self.value_with_preferences(&ProjectPreferences::default())?;
        let config: Config = value
            .try_into()
            .map_err(|_| config_error("type", self.workspace()))?;
        Ok(
            (config.provider == "responses").then_some(ResponsesRouteConfig {
                api_base: config.api_base,
                api_key_env: config.api_key_env,
            }),
        )
    }

    /// Parseable effective TOML before saved project preferences are loaded.
    pub fn snapshot_toml(&self) -> Result<String> {
        let (mut value, _) = self.value_with_preferences(&ProjectPreferences::default())?;
        if let Some(mcp) = value.get_mut("mcp").and_then(toml::Value::as_table_mut) {
            for server in mcp.iter_mut().map(|(_, server)| server) {
                if let Some(environment) = server.get_mut("env").and_then(toml::Value::as_table_mut)
                {
                    for value in environment.iter_mut().map(|(_, value)| value) {
                        *value = "[redacted]".into();
                    }
                }
            }
        }
        toml::to_string_pretty(&value).map_err(|_| config_error("type", self.workspace()))
    }

    pub fn finalize(&self, preferences: &ProjectPreferences) -> Result<Config> {
        let (value, _) = self.value_with_preferences(preferences)?;
        let config: Config = value
            .try_into()
            .map_err(|_| config_error("type", self.workspace()))?;
        config
            .validate()
            .map_err(|_| config_error("validation", self.workspace()))?;
        Ok(config)
    }

    fn value_with_preferences(
        &self,
        preferences: &ProjectPreferences,
    ) -> Result<(toml::Value, BTreeMap<String, LayerOrigin>)> {
        let mut value = self.merged.clone();
        let mut origins = self.origins.clone();
        let provider = self
            .overrides
            .provider
            .as_deref()
            .or_else(|| self.local.as_ref()?.get("provider")?.as_str())
            .or_else(|| value.get("provider")?.as_str())
            .ok_or_else(|| config_error("type", self.workspace()))?
            .to_owned();
        let explicit_model = self
            .overrides
            .model
            .as_deref()
            .or_else(|| self.local.as_ref()?.get("model")?.as_str())
            .map(str::to_owned);
        preferences
            .overlay(&mut value, &provider, explicit_model.as_deref())
            .map_err(|_| config_error("validation", self.workspace()))?;
        if let Some(local) = &self.local {
            merge_with_origins(
                &mut value,
                &mut origins,
                local.clone(),
                LayerOrigin {
                    source: SafeSource("explicit configuration".into()),
                    source_digest: source_digest(b"explicit configuration"),
                    automatic: false,
                },
            );
            origins.extend(self.local_origins.clone());
        }
        apply_overrides(&mut value, &mut origins, &self.overrides);
        Ok((value, origins))
    }
}

impl Config {
    /// Decide a checked invocation. The caller must have validated the physical
    /// workspace root, protected paths and file target before using this result.
    pub fn permission_decision(
        &self,
        selector: &PermissionSelector,
        target: Option<&ProjectRelativeTarget>,
    ) -> PermissionAction {
        match selector {
            PermissionSelector::Mcp { alias, .. } if !self.mcp.contains_key(alias) => {
                return PermissionAction::Deny;
            }
            PermissionSelector::A2a { alias } if !self.external_agents.contains_key(alias) => {
                return PermissionAction::Deny;
            }
            _ => {}
        }
        permissions::decide(
            &self.permissions,
            selector,
            target,
            self.allow_write,
            self.allow_shell,
        )
    }

    /// Merge user, outer-to-inner project, then explicit local TOML.
    ///
    /// Missing ancestor files are normal. An explicitly provided user or local
    /// path must exist. Maps merge recursively and arrays replace. Each layer is
    /// checked for unknown keys and field types; semantic checks run after merging.
    pub fn load(user: Option<&Path>, project: &Path, local: Option<&Path>) -> Result<Self> {
        ConfigSnapshot::parse(user, project, local, InvocationOverrides::default())
            .and_then(|snapshot| snapshot.finalize(&ProjectPreferences::default()))
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
        ConfigSnapshot::parse(user, project, local, overrides.into())?.finalize(preferences)
    }

    /// Storage bootstrap is independent of choices stored inside that storage.
    pub fn load_memory(
        user: Option<&Path>,
        project: &Path,
        local: Option<&Path>,
    ) -> Result<MemoryConfig> {
        Ok(
            ConfigSnapshot::parse(user, project, local, InvocationOverrides::default())?
                .memory_config()
                .clone(),
        )
    }

    pub fn validate(&self) -> Result<()> {
        self.memory.validate()?;
        permissions::validate_rules(&self.permissions)?;
        for rule in &self.permissions {
            match &rule.selector {
                PermissionSelector::Mcp { alias, .. } => {
                    ensure!(
                        self.mcp.contains_key(alias),
                        "permission MCP alias is not configured"
                    );
                }
                PermissionSelector::A2a { alias } => {
                    ensure!(
                        self.external_agents.contains_key(alias),
                        "permission A2A alias is not configured"
                    );
                }
                PermissionSelector::Native { .. } => {}
            }
        }
        ensure!(
            matches!(self.provider.as_str(), "codex" | "responses" | "demo"),
            "provider must be codex, responses, or demo"
        );
        nonempty("model", &self.model, 256)?;
        if let Some(effort) = &self.effort {
            nonempty("effort", effort, 128)?;
        }
        if let Some(window) = self.assumed_context_window_tokens {
            ensure!(
                (1..=2_000_000).contains(&window),
                "assumed_context_window_tokens must be between 1 and 2000000"
            );
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

fn reject_removed_settings(patch: &toml::Value) -> Result<()> {
    ensure!(
        patch.get("codex_command").is_none(),
        "codex_command was removed: Kuru connects to OpenAI directly; remove this setting and use `kuru login`, or select the responses provider with OPENAI_API_KEY"
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

fn config_error(category: &str, path: &Path) -> anyhow::Error {
    anyhow::anyhow!("configuration {category} error in {}", safe_source(path))
}

fn config_parse_error(path: &Path, source: &str, error: &toml::de::Error) -> anyhow::Error {
    let position = error
        .span()
        .map(|span| {
            let prefix = &source[..span.start.min(source.len())];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = prefix
                .rsplit_once('\n')
                .map_or(prefix.chars().count() + 1, |(_, tail)| {
                    tail.chars().count() + 1
                });
            format!(" at line {line}, column {column}")
        })
        .unwrap_or_default();
    anyhow::anyhow!(
        "configuration parse error in {}{position}",
        safe_source(path)
    )
}

fn safe_source(path: &Path) -> SafeSource {
    let text = path.as_os_str().to_string_lossy();
    let escaped = text
        .chars()
        .flat_map(char::escape_default)
        .take(160)
        .collect::<String>();
    SafeSource(if escaped.is_empty() {
        "configuration source".into()
    } else {
        escaped
    })
}

fn source_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"kuru.workspace-trust.source\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    hash.finalize().into()
}

fn layer_origin(path: &Path, automatic: bool) -> LayerOrigin {
    LayerOrigin {
        source: safe_source(path),
        source_digest: source_digest(path.as_os_str().as_encoded_bytes()),
        automatic,
    }
}

fn read_config_bounded(path: &Path, required: bool) -> Result<Option<String>> {
    read_bounded(path, required).map_err(|_| config_error("read", path))
}

fn merge_with_origins(
    base: &mut toml::Value,
    origins: &mut BTreeMap<String, LayerOrigin>,
    patch: toml::Value,
    origin: LayerOrigin,
) {
    merge(base, patch.clone());
    record_origins(&patch, "", origin, origins);
}

fn record_origins(
    value: &toml::Value,
    prefix: &str,
    origin: LayerOrigin,
    origins: &mut BTreeMap<String, LayerOrigin>,
) {
    if let toml::Value::Table(table) = value {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            record_origins(value, &path, origin.clone(), origins);
        }
    } else {
        origins.insert(prefix.into(), origin);
    }
}

fn apply_overrides(
    value: &mut toml::Value,
    origins: &mut BTreeMap<String, LayerOrigin>,
    overrides: &InvocationOverrides,
) {
    let table = value
        .as_table_mut()
        .expect("configuration default is a table");
    let cli = || LayerOrigin {
        source: SafeSource("command line".into()),
        source_digest: source_digest(b"command line"),
        automatic: false,
    };
    if let Some(mode) = overrides.mode {
        table.insert(
            "mode".into(),
            toml::Value::try_from(mode).expect("mode serializes"),
        );
        origins.insert("mode".into(), cli());
    }
    if let Some(provider) = &overrides.provider {
        table.insert("provider".into(), provider.clone().into());
        origins.insert("provider".into(), cli());
    }
    if let Some(model) = &overrides.model {
        table.insert("model".into(), model.clone().into());
        origins.insert("model".into(), cli());
    }
    if let Some(effort) = &overrides.effort {
        table.insert("effort".into(), effort.clone().into());
        origins.insert("effort".into(), cli());
    }
    if overrides.allow_write {
        table.insert("allow_write".into(), true.into());
        origins.insert("allow_write".into(), cli());
    }
    if overrides.allow_shell {
        table.insert("allow_shell".into(), true.into());
        origins.insert("allow_shell".into(), cli());
    }
    if overrides.no_dream {
        table.insert("dream_every".into(), 0.into());
        table.insert("dream_on_exit".into(), false.into());
    }
}

fn empty_manifest() -> AuthorityManifest {
    SafeManifest::new(1, Vec::new()).into_manifest()
}

impl SafeManifest {
    fn into_manifest(self) -> AuthorityManifest {
        AuthorityManifest {
            schema_version: self.schema_version,
            digest: self.digest,
            claims: self.claims,
            sources: self.sources,
        }
    }
}

fn manifest_digest(schema_version: u16, claims: &[AuthorityClaim]) -> ManifestDigest {
    let mut hash = Sha256::new();
    hash.update(b"kuru.workspace-trust.manifest\0");
    hash.update(schema_version.to_be_bytes());
    for claim in claims {
        hash.update((claim.category as u8).to_be_bytes());
        hash.update(claim.digest.0);
        for source_digest in &claim.source_digests {
            hash.update(source_digest);
        }
    }
    ManifestDigest(hash.finalize().into())
}

fn claim_digest(category: AuthorityClaimCategory, value: impl Serialize) -> Result<ClaimDigest> {
    // All claim values are concrete, serde-serializable configuration types.
    // JSON preserves BTreeMap order and represents root scalars and tuples.
    let encoded = serde_json::to_vec(&value)
        .map_err(|_| anyhow::anyhow!("canonical authority encoding failed"))?;
    let mut hash = Sha256::new();
    hash.update(b"kuru.workspace-trust.claim\0");
    hash.update((category as u8).to_be_bytes());
    hash.update((encoded.len() as u64).to_be_bytes());
    hash.update(encoded);
    Ok(ClaimDigest(hash.finalize().into()))
}

fn automatic_origins<'a>(
    origins: &'a BTreeMap<String, LayerOrigin>,
    path: &str,
) -> Vec<&'a LayerOrigin> {
    let prefix = format!("{path}.");
    origins
        .iter()
        .filter_map(|(key, origin)| {
            ((key == path || key.starts_with(&prefix)) && origin.automatic).then_some(origin)
        })
        .collect()
}

fn push_claim(
    claims: &mut Vec<AuthorityClaim>,
    category: AuthorityClaimCategory,
    value: impl Serialize,
    origins: &[&LayerOrigin],
    display: impl Into<String>,
) -> Result<()> {
    let sources = origins
        .iter()
        .map(|origin| origin.source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let source_digests = origins
        .iter()
        .map(|origin| origin.source_digest)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let source = sources
        .first()
        .expect("claim has an automatic source")
        .clone();
    claims.push(AuthorityClaim {
        category,
        digest: claim_digest(category, value)?,
        source,
        sources,
        source_digests,
        display: SafeClaimDisplay(display.into()),
    });
    Ok(())
}

fn derive_manifest(
    config: &Config,
    origins: &BTreeMap<String, LayerOrigin>,
    instruction_sources: &[InstructionSource],
) -> Result<AuthorityManifest> {
    let mut claims = Vec::new();
    let write_origins = automatic_origins(origins, "allow_write");
    if config.allow_write && !write_origins.is_empty() {
        push_claim(
            &mut claims,
            AuthorityClaimCategory::WorkspaceWrite,
            config.allow_write,
            &write_origins,
            "workspace write enabled",
        )?;
    }
    let shell_origins = automatic_origins(origins, "allow_shell");
    if config.allow_shell && !shell_origins.is_empty() {
        push_claim(
            &mut claims,
            AuthorityClaimCategory::Shell,
            config.allow_shell,
            &shell_origins,
            "shell enabled",
        )?;
    }
    let permission_origins = automatic_origins(origins, "permissions");
    if !permission_origins.is_empty() {
        push_claim(
            &mut claims,
            AuthorityClaimCategory::ToolPermissions,
            &config.permissions,
            &permission_origins,
            format!(
                "{} ordered tool permission rule(s)",
                config.permissions.len()
            ),
        )?;
    }
    let binary_origins = automatic_origins(origins, "memory.dolt_binary");
    if config.memory.dolt_binary.is_some() && !binary_origins.is_empty() {
        push_claim(
            &mut claims,
            AuthorityClaimCategory::MemoryDoltBinary,
            &config.memory.dolt_binary,
            &binary_origins,
            "configured memory executable",
        )?;
    }
    let cache_origins = automatic_origins(origins, "memory.cache_dir");
    if config.memory.cache_dir.is_some() && !cache_origins.is_empty() {
        push_claim(
            &mut claims,
            AuthorityClaimCategory::MemoryCacheDir,
            &config.memory.cache_dir,
            &cache_origins,
            "configured memory cache",
        )?;
    }
    if config.provider == "responses" {
        let response_origins = ["provider", "api_base", "api_key_env"]
            .into_iter()
            .flat_map(|path| automatic_origins(origins, path))
            .collect::<Vec<_>>();
        if !response_origins.is_empty() {
            push_claim(
                &mut claims,
                AuthorityClaimCategory::ResponsesRoute,
                (&config.provider, &config.api_base, &config.api_key_env),
                &response_origins,
                "Responses credential route",
            )?;
        }
    }
    for (name, mcp) in &config.mcp {
        let path = format!("mcp.{name}");
        let mcp_origins = automatic_origins(origins, &path);
        if !mcp_origins.is_empty() {
            let category = if mcp.command.is_some() {
                AuthorityClaimCategory::McpStdio
            } else {
                AuthorityClaimCategory::McpHttp
            };
            let display_name = safe_text(name, 64);
            if mcp.command.is_some() {
                push_claim(
                    &mut claims,
                    category,
                    (name, &mcp.command, &mcp.args, &mcp.env),
                    &mcp_origins,
                    format!("{display_name}: configured executable"),
                )?;
            } else {
                push_claim(
                    &mut claims,
                    category,
                    (name, &mcp.url),
                    &mcp_origins,
                    format!("{display_name}: HTTP endpoint"),
                )?;
            }
        }
    }
    for (name, url) in &config.external_agents {
        let agent_origins = automatic_origins(origins, &format!("external_agents.{name}"));
        if !agent_origins.is_empty() {
            push_claim(
                &mut claims,
                AuthorityClaimCategory::ExternalAgent,
                (name, url),
                &agent_origins,
                format!("{}: external agent", safe_text(name, 64)),
            )?;
        }
    }
    if !instruction_sources.is_empty() {
        let entries = instruction_sources
            .iter()
            .map(|source| InstructionClaimEntry {
                kind: source.kind,
                path_digest: &source.path_digest,
                identity: &source.identity,
                content: &source.content,
            })
            .collect::<Vec<_>>();
        let sources = instruction_sources
            .iter()
            .map(|source| source.safe_source.clone())
            .collect::<Vec<_>>();
        let source_digests = instruction_sources
            .iter()
            .map(|source| source_digest_with_identity(source.path_digest, source.identity))
            .collect::<Vec<_>>();
        claims.push(AuthorityClaim {
            category: AuthorityClaimCategory::ProjectInstructions,
            digest: claim_digest(AuthorityClaimCategory::ProjectInstructions, &entries)?,
            source: sources
                .first()
                .expect("instruction claim has a source")
                .clone(),
            sources,
            source_digests,
            display: SafeClaimDisplay(format!(
                "{} ordered automatic source{}",
                instruction_sources.len(),
                if instruction_sources.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )),
        });
    }
    claims.sort_by_key(|claim| (claim.category, claim.digest));
    let mut manifest = SafeManifest::new(1, claims).into_manifest();
    manifest.sources = manifest
        .claims
        .iter()
        .flat_map(|claim| claim.sources.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(manifest)
}

fn source_digest_with_identity(path_digest: [u8; 32], identity: [u8; 24]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"kuru.workspace-trust.instruction-source\0");
    hash.update(path_digest);
    hash.update(identity);
    hash.finalize().into()
}

fn safe_text(value: &str, max: usize) -> String {
    value
        .chars()
        .flat_map(char::escape_default)
        .take(max)
        .collect()
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
    // Windows rejects opening a directory before handle metadata is available.
    // Classify ordinary wrong-type inputs first, then validate the opened file
    // again so this pathname check is never the authority for the actual read.
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound && !required => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    ensure!(
        metadata.is_file(),
        "{} must be a regular file",
        path.display()
    );
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

fn capture_instruction_sources(project: &Path) -> Result<Vec<InstructionSource>> {
    let mut sources = Vec::new();
    let mut total_bytes: usize = 0;
    for directory in ancestor_directories(project)? {
        let path = directory.join("AGENTS.md");
        let Some((source, identity)) = read_instruction_bounded(&path)? else {
            continue;
        };
        total_bytes = total_bytes
            .checked_add(source.len())
            .ok_or_else(|| instruction_error("read", &path))?;
        ensure!(
            total_bytes <= MAX_COMBINED_BYTES,
            "combined AGENTS.md instructions exceed 1 MiB"
        );
        sources.push(InstructionSource {
            kind: InstructionSourceKind::Agents,
            path_digest: source_digest(path.as_os_str().as_encoded_bytes()),
            safe_source: safe_source(&path),
            path,
            identity,
            content: source,
        });
    }
    Ok(sources)
}

fn read_instruction_bounded(path: &Path) -> Result<Option<(String, [u8; 24])>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(instruction_error("read", path)),
    };
    ensure!(
        metadata.is_file(),
        "{} must be a regular file",
        safe_source(path)
    );
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(instruction_error("read", path)),
    };
    let info =
        kuru_platform::fs::regular_file_info(&file).map_err(|_| instruction_error("read", path))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| instruction_error("read", path))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(instruction_error("read", path));
    }
    let source = String::from_utf8(bytes).map_err(|_| instruction_error("encoding", path))?;
    Ok(Some((source, info.identity.to_bytes())))
}

fn instruction_error(category: &str, path: &Path) -> anyhow::Error {
    anyhow::anyhow!("instruction {category} error in {}", safe_source(path))
}

fn format_instructions(sources: &[InstructionSource]) -> String {
    let mut combined = String::new();
    if !sources.is_empty() {
        combined.push_str("Project instructions follow from outermost to most local. Where instructions conflict, the most local applicable AGENTS.md takes precedence; higher-priority conversation instructions still apply.\n");
    }
    for source in sources {
        combined.push_str(&format!(
            "\n--- {}: {} ---\n",
            source.kind.prompt_name(),
            source.path.display()
        ));
        combined.push_str(&source.content);
        combined.push('\n');
    }
    combined
}

/// Include every ancestor AGENTS.md, clearly identifying increasingly local scope.
/// Source order conveys precedence without attempting to reinterpret instructions.
pub fn load_instructions(project: &Path) -> Result<String> {
    capture_instruction_sources(project).map(|sources| format_instructions(&sources))
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn memory_options_validate_paths_timeouts_and_unknown_fields() {
        assert_eq!(MemoryConfig::default().startup_timeout_secs, 30);
        for timeout in [1, 300] {
            MemoryConfig {
                startup_timeout_secs: timeout,
                ..MemoryConfig::default()
            }
            .validate()
            .unwrap();
        }
        for timeout in [0, 301, u64::MAX] {
            assert!(
                MemoryConfig {
                    startup_timeout_secs: timeout,
                    ..MemoryConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        for path in ["", "bad\0path"] {
            assert!(
                MemoryConfig {
                    dolt_binary: Some(path.into()),
                    ..MemoryConfig::default()
                }
                .validate()
                .is_err()
            );
            assert!(
                MemoryConfig {
                    cache_dir: Some(path.into()),
                    ..MemoryConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        for path in ["dolt", "./dolt", "../dolt"] {
            assert!(
                MemoryConfig {
                    dolt_binary: Some(path.into()),
                    ..MemoryConfig::default()
                }
                .validate()
                .is_err()
            );
            assert!(
                MemoryConfig {
                    cache_dir: Some(path.into()),
                    ..MemoryConfig::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(toml::from_str::<MemoryConfig>("unknown_option = true").is_err());
        assert!(toml::from_str::<MemoryConfig>("offline = 'yes'").is_err());
        let directory = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            offline: true,
            cache_dir: Some(directory.path().join("cache-not-created")),
            dolt_binary: Some(directory.path().join("dolt-not-created")),
            ..MemoryConfig::default()
        };
        config.validate().unwrap();
        assert!(config.offline);
        assert!(!config.cache_dir.as_ref().unwrap().exists());
        assert!(!config.dolt_binary.as_ref().unwrap().exists());
    }

    #[test]
    fn memory_bootstrap_merges_files_without_validating_unloaded_saved_mode() {
        let directory = tempfile::tempdir().unwrap();
        let user = directory.path().join("user.toml");
        let project = directory.path().join("project");
        let shared_cache = directory.path().join("shared-cache");
        std::fs::create_dir_all(project.join(".kuru")).unwrap();
        std::fs::write(
            &user,
            format!(
                "max_parts = 3\n[memory]\ncache_dir = {}\nstartup_timeout_secs = 12\n",
                toml::Value::String(shared_cache.to_string_lossy().into_owned())
            ),
        )
        .unwrap();
        std::fs::write(
            project.join(".kuru/config.toml"),
            "[memory]\noffline = true\n",
        )
        .unwrap();
        let memory = Config::load_memory(Some(&user), &project, None).unwrap();
        assert!(memory.offline);
        assert_eq!(memory.startup_timeout_secs, 12);
        assert_eq!(memory.cache_dir.as_deref(), Some(shared_cache.as_path()));
        // IFS needs more than three parts, but the stored Freudian choice is valid.
        assert!(Config::load(Some(&user), &project, None).is_err());
        let config = Config::load_with_preferences(
            Some(&user),
            &project,
            None,
            &ProjectPreferences {
                mode: Some(Mode::Freudian),
                ..ProjectPreferences::default()
            },
            SelectionOverrides::default(),
        )
        .unwrap();
        assert_eq!(config.mode, Mode::Freudian);
        assert_eq!(config.memory, memory);
        let local = directory.path().join("local.toml");
        std::fs::write(
            &local,
            "[memory]\noffline = false\nstartup_timeout_secs = 0\n",
        )
        .unwrap();
        assert!(Config::load_memory(Some(&user), &project, Some(&local)).is_err());
        std::fs::write(
            &local,
            "[memory]\noffline = false\nstartup_timeout_secs = 300\n",
        )
        .unwrap();
        let memory = Config::load_memory(Some(&user), &project, Some(&local)).unwrap();
        assert!(!memory.offline);
        assert_eq!(memory.startup_timeout_secs, 300);
    }
}
