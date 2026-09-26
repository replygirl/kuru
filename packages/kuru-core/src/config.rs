use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::prompt_sources::{
    MAX_MATERIAL_SOURCES, MAX_MATERIAL_TOTAL, MaterialKind, PromptCatalog, PromptMaterial,
    select_material,
};
use crate::{
    Framework, Mode, PermissionAction, PermissionRule, PermissionSelector, ProjectRelativeTarget,
};
use crate::{
    context::{
        DEFAULT_COMPACTION_OUTPUT_RESERVE_TOKENS, DEFAULT_COMPACTION_THRESHOLD_PERCENT,
        MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS,
    },
    permissions,
};

const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_COMBINED_BYTES: usize = 1024 * 1024;
const MAX_INSTRUCTION_PATHS: usize = 128;
const MAX_IMPORT_DEPTH: usize = 8;
const MAX_INSTRUCTION_NOTICES: usize = 16;
pub const MAX_HOOKS_PER_EVENT: usize = 16;
pub const MAX_HOOK_ARGUMENTS: usize = 64;
pub const MAX_HOOK_TIMEOUT_MS: u64 = 120_000;
pub const MAX_HOOK_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_HOOK_INVOCATIONS: usize = 1024;
pub const MAX_HOOK_TOTAL_MS: u64 = 600_000;
pub const MAX_HOOK_ANNOTATION_BYTES: usize = 1024 * 1024;

const fn default_hook_timeout_ms() -> u64 {
    5_000
}

const fn default_hook_output_bytes() -> usize {
    64 * 1024
}

const fn default_hook_invocations() -> usize {
    256
}
const fn default_hook_total_ms() -> u64 {
    120_000
}
const fn default_hook_annotation_bytes() -> usize {
    256 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HookCommand {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

impl Default for HookCommand {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            timeout_ms: default_hook_timeout_ms(),
            max_output_bytes: default_hook_output_bytes(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreTurn,
    PostTurn,
    PreTool,
    PostTool,
    SpeakerSelected,
}

impl HookEvent {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PreTurn => "pre_turn",
            Self::PostTurn => "post_turn",
            Self::PreTool => "pre_tool",
            Self::PostTool => "post_tool",
            Self::SpeakerSelected => "speaker_selected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LifecycleHooks {
    pub max_invocations: usize,
    pub max_total_ms: u64,
    pub max_annotation_bytes: usize,
    pub pre_turn: Vec<HookCommand>,
    pub post_turn: Vec<HookCommand>,
    pub pre_tool: Vec<HookCommand>,
    pub post_tool: Vec<HookCommand>,
    pub speaker_selected: Vec<HookCommand>,
}

impl Default for LifecycleHooks {
    fn default() -> Self {
        Self {
            max_invocations: default_hook_invocations(),
            max_total_ms: default_hook_total_ms(),
            max_annotation_bytes: default_hook_annotation_bytes(),
            pre_turn: Vec::new(),
            post_turn: Vec::new(),
            pre_tool: Vec::new(),
            post_tool: Vec::new(),
            speaker_selected: Vec::new(),
        }
    }
}

impl LifecycleHooks {
    pub fn event(&self, event: HookEvent) -> &[HookCommand] {
        match event {
            HookEvent::PreTurn => &self.pre_turn,
            HookEvent::PostTurn => &self.post_turn,
            HookEvent::PreTool => &self.pre_tool,
            HookEvent::PostTool => &self.post_tool,
            HookEvent::SpeakerSelected => &self.speaker_selected,
        }
    }

    pub fn is_empty(&self) -> bool {
        [
            &self.pre_turn,
            &self.post_turn,
            &self.pre_tool,
            &self.post_tool,
            &self.speaker_selected,
        ]
        .into_iter()
        .all(Vec::is_empty)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_HOOK_INVOCATIONS).contains(&self.max_invocations),
            "hooks.max_invocations must be between 1 and {MAX_HOOK_INVOCATIONS}"
        );
        ensure!(
            (1..=MAX_HOOK_TOTAL_MS).contains(&self.max_total_ms),
            "hooks.max_total_ms must be between 1 and {MAX_HOOK_TOTAL_MS}"
        );
        ensure!(
            (1..=MAX_HOOK_ANNOTATION_BYTES).contains(&self.max_annotation_bytes),
            "hooks.max_annotation_bytes must be between 1 and {MAX_HOOK_ANNOTATION_BYTES}"
        );
        for event in [
            HookEvent::PreTurn,
            HookEvent::PostTurn,
            HookEvent::PreTool,
            HookEvent::PostTool,
            HookEvent::SpeakerSelected,
        ] {
            let hooks = self.event(event);
            ensure!(
                hooks.len() <= MAX_HOOKS_PER_EVENT,
                "hooks.{} has too many commands",
                event.label()
            );
            for hook in hooks {
                nonempty("hook command", &hook.command, 4096)?;
                ensure!(
                    !hook.command.contains('\0'),
                    "hook command cannot contain NUL characters"
                );
                ensure!(
                    hook.args.len() <= MAX_HOOK_ARGUMENTS,
                    "hook command has too many arguments"
                );
                ensure!(
                    hook.args
                        .iter()
                        .all(|argument| argument.len() <= 4096 && !argument.contains('\0')),
                    "hook arguments must be at most 4096 bytes without NUL characters"
                );
                ensure!(
                    (1..=MAX_HOOK_TIMEOUT_MS).contains(&hook.timeout_ms),
                    "hook timeout_ms must be between 1 and {MAX_HOOK_TIMEOUT_MS}"
                );
                ensure!(
                    (1..=MAX_HOOK_OUTPUT_BYTES).contains(&hook.max_output_bytes),
                    "hook max_output_bytes must be between 1 and {MAX_HOOK_OUTPUT_BYTES}"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    pub enabled: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
    /// HTTP header names mapped to environment-variable names. Resolved header
    /// values never enter the configuration snapshot.
    pub header_env: BTreeMap<String, String>,
    pub oauth: Option<McpOAuthConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: None,
            args: Vec::new(),
            url: None,
            env: BTreeMap::new(),
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            header_env: BTreeMap::new(),
            oauth: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct McpOAuthConfig {
    pub enabled: bool,
    pub client_id: Option<String>,
    pub client_secret_env: Option<String>,
    pub client_metadata_url: Option<String>,
    pub scopes: Vec<String>,
}

impl McpConfig {
    /// Apply deny-first original MCP tool-name filtering. This is catalog
    /// admission only; it is never an execution grant.
    pub fn admits_tool(&self, name: &str) -> bool {
        !self
            .deny_tools
            .iter()
            .any(|pattern| mcp_tool_glob_matches(pattern, name))
            && (self.allow_tools.is_empty()
                || self
                    .allow_tools
                    .iter()
                    .any(|pattern| mcp_tool_glob_matches(pattern, name)))
    }
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
    /// Typed `-c key=value` assignments, in command-line order.
    pub typed_config: Vec<String>,
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
    ProjectSkillMetadata,
    ProjectSkillMaterial,
    ProjectCommands,
    LifecycleHooks,
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
            Self::ProjectSkillMetadata => "project skill metadata",
            Self::ProjectSkillMaterial => "selected project skill material",
            Self::ProjectCommands => "project commands",
            Self::LifecycleHooks => "lifecycle hooks",
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
    /// Optional fit reserve. This is local accounting, not a provider wire
    /// parameter or an override of verified model output metadata.
    pub context_output_reserve_tokens: Option<u64>,
    /// Percentage of the effective model window at which Kuru attempts one
    /// actor-local rolling context compaction before ordinary fit omission.
    pub context_compaction_threshold_percent: u8,
    /// Output space reserved for the compaction provider request. This is
    /// independent of the ordinary answer reserve.
    pub context_compaction_output_reserve_tokens: u64,
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
    pub hooks: LifecycleHooks,
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
            context_output_reserve_tokens: None,
            context_compaction_threshold_percent: DEFAULT_COMPACTION_THRESHOLD_PERCENT,
            context_compaction_output_reserve_tokens: DEFAULT_COMPACTION_OUTPUT_RESERVE_TOKENS,
            max_rounds: 3,
            max_tool_calls: 12,
            max_parallel: 4,
            dream_every: 8,
            dream_on_exit: true,
            max_parts: 16,
            allow_shell: false,
            allow_write: false,
            permissions: Vec::new(),
            hooks: LifecycleHooks::default(),
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
    discovered_local: Option<toml::Value>,
    discovered_local_origins: BTreeMap<String, LayerOrigin>,
    constraints: Vec<(Vec<String>, toml::Value)>,
    overrides: InvocationOverrides,
    manifest: AuthorityManifest,
    memory: MemoryConfig,
    instructions: String,
    instruction_notices: Vec<String>,
    instruction_sources: Vec<InstructionSource>,
    instruction_cache: BTreeMap<PathBuf, Option<CheckedInstruction>>,
    base_instruction_cache_paths: BTreeSet<PathBuf>,
    instruction_directories: BTreeSet<PathBuf>,
    instruction_directory_identities: BTreeMap<PathBuf, [u8; 24]>,
    base_instruction_paths: BTreeSet<PathBuf>,
    instruction_body: String,
    prompt_catalog: PromptCatalog,
    selected_material: Vec<PromptMaterial>,
    user_prompt_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct LayerOrigin {
    source: SafeSource,
    source_digest: [u8; 32],
    automatic: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InstructionSourceKind {
    Agents,
    Claude,
    Import,
}

impl InstructionSourceKind {
    const fn prompt_name(self) -> &'static str {
        match self {
            Self::Agents => "AGENTS.md",
            Self::Claude => "CLAUDE.md",
            Self::Import => "imported Markdown",
        }
    }
}

#[derive(Debug, Clone)]
struct InstructionSource {
    path: PathBuf,
    kind: InstructionSourceKind,
    safe_source: SafeSource,
    path_digest: [u8; 32],
    directory_identity: [u8; 24],
    identity: [u8; 24],
    content: String,
}

#[derive(Serialize)]
struct InstructionClaimEntry<'a> {
    kind: InstructionSourceKind,
    path_digest: &'a [u8; 32],
    directory_identity: &'a [u8; 24],
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
        Self::parse_with_layers(user, workspace, None, local, None, overrides)
    }

    /// All file sources are caller-classified before this immutable snapshot is
    /// built. `discovered_local` must contain the exact checked bytes classified
    /// by the CLI's Git index check, not a path to reopen later.
    pub fn parse_with_layers(
        user: Option<&Path>,
        workspace: &Path,
        discovered_local: Option<(&Path, &str)>,
        local: Option<&Path>,
        managed: Option<&Path>,
        overrides: InvocationOverrides,
    ) -> Result<Self> {
        Self::parse_with_sources(
            user,
            user.and_then(Path::parent),
            workspace,
            discovered_local,
            local,
            managed,
            &[],
            overrides,
        )
    }

    /// The application supplies its built-in command names so a shadowed
    /// repository prompt never becomes effective authority.
    #[allow(clippy::too_many_arguments)]
    pub fn parse_with_sources(
        user: Option<&Path>,
        user_prompt_root: Option<&Path>,
        workspace: &Path,
        discovered_local: Option<(&Path, &str)>,
        local: Option<&Path>,
        managed: Option<&Path>,
        built_in_commands: &[&str],
        overrides: InvocationOverrides,
    ) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| config_error("read", workspace))?;
        if !workspace.is_dir() {
            return Err(config_error("read", &workspace));
        }
        let mut instruction_cache = BTreeMap::new();
        let InstructionCapture {
            sources: instruction_sources,
            rendered: instructions,
            notices: instruction_notices,
            ..
        } = capture_instruction_sources(&workspace, &[], &mut instruction_cache)?;
        let base_instruction_paths = instruction_sources
            .iter()
            .map(|source| source.path.clone())
            .collect();
        let base_instruction_cache_paths = instruction_cache.keys().cloned().collect();
        let prompt_catalog =
            PromptCatalog::discover(&workspace, user_prompt_root, built_in_commands)?;
        let rendered_instructions = render_prompt_instructions(&instructions, &prompt_catalog, &[]);
        let mut merged =
            toml::Value::try_from(Config::default()).expect("default config serializes");
        let mut origins = BTreeMap::new();
        let mut total_bytes: usize = 0;
        let mut layers = Vec::new();
        let constraints = if let Some(path) = managed {
            let canonical = path
                .canonicalize()
                .map_err(|_| config_error("read", path))?;
            ensure!(
                path.is_absolute() && !canonical.starts_with(&workspace),
                "managed configuration must be an absolute file outside the workspace"
            );
            let source = read_config_bounded(&canonical, true)?
                .ok_or_else(|| config_error("read", &canonical))?;
            total_bytes = total_bytes
                .checked_add(source.len())
                .ok_or_else(|| config_error("read", &canonical))?;
            let policy: ManagedDocument = toml::from_str(&source)
                .map_err(|error| config_parse_error(&canonical, &source, &error))?;
            let defaults = policy.defaults.unwrap_or_else(empty_table);
            validate_patch(&defaults, &canonical)?;
            merge_with_origins(
                &mut merged,
                &mut origins,
                defaults,
                layer_origin(&canonical, false),
            );
            let constraints = policy.constraints.unwrap_or_else(empty_table);
            validate_patch(&constraints, &canonical)?;
            let mut locks = Vec::new();
            collect_constraints(&constraints, &[], &mut locks);
            let mut normalized = toml::Value::try_from(Config::default())?;
            merge(&mut normalized, constraints);
            let typed: Config = normalized
                .try_into()
                .map_err(|error| config_type_error(&canonical, &error))?;
            let normalized = toml::Value::try_from(typed)?;
            for (path, expected) in &mut locks {
                *expected = lookup_path(&normalized, path)
                    .ok_or_else(|| config_error("type", &canonical))?
                    .clone();
            }
            locks
        } else {
            Vec::new()
        };
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
                .map_err(|error| config_type_error(&path, &error))?;
        }
        let (discovered_local, discovered_local_origins) =
            parse_captured_patch(discovered_local, &merged, &mut total_bytes)?;
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
            let _: Config = checked
                .try_into()
                .map_err(|error| config_type_error(path, &error))?;
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
            discovered_local,
            discovered_local_origins,
            constraints,
            overrides,
            manifest: empty_manifest(),
            memory: MemoryConfig::default(),
            instructions: rendered_instructions,
            instruction_notices,
            instruction_sources: instruction_sources.clone(),
            instruction_cache,
            base_instruction_cache_paths,
            instruction_directories: BTreeSet::new(),
            instruction_directory_identities: BTreeMap::new(),
            base_instruction_paths,
            instruction_body: instructions,
            prompt_catalog,
            selected_material: Vec::new(),
            user_prompt_root: user_prompt_root.map(Path::to_path_buf),
        };
        let (value, value_origins) =
            provisional.value_with_preferences(&ProjectPreferences::default())?;
        let config: Config = value
            .clone()
            .try_into()
            .map_err(|error| config_type_error(provisional.workspace(), &error))?;
        config
            .memory
            .validate()
            .map_err(|_| config_error("validation", provisional.workspace()))?;
        permissions::validate_rules(&config.permissions)
            .map_err(|_| config_error("validation", provisional.workspace()))?;
        config
            .hooks
            .validate()
            .map_err(|_| config_error("validation", provisional.workspace()))?;
        provisional.check_constraints(&config, false)?;
        let manifest = derive_manifest(
            &config,
            &value_origins,
            &instruction_sources,
            &provisional.prompt_catalog,
            &provisional.selected_material,
        )?;
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

    /// Bounded, escaped notices for instruction sources omitted during capture.
    pub fn instruction_notices(&self) -> &[String] {
        &self.instruction_notices
    }

    /// Derive a new immutable prompt-authority snapshot for checked,
    /// project-relative target directories. Previously active source bytes and
    /// identities retain their captured state for this invocation. Directories
    /// without active nested sources are not retained across tool calls.
    /// The caller must review the returned manifest before using its prompt.
    pub fn with_nested_directories(&self, directories: &[PathBuf]) -> Result<(Self, bool)> {
        ensure!(
            directories.len() <= 10_000,
            "nested instruction target set exceeds the 10000-candidate limit"
        );
        let root = kuru_platform::fs::Directory::open(
            &self.workspace,
            kuru_platform::fs::Privacy::Inherited,
            kuru_platform::fs::NameRetention::Pinned,
        )
        .map_err(|_| instruction_error("path", &self.workspace))?;
        let mut selected = self.instruction_directories.clone();
        for relative in directories {
            ensure!(
                !relative.is_absolute(),
                "nested instruction target must be project-relative"
            );
            let mut path = self.workspace.clone();
            for component in relative.components() {
                match component {
                    Component::CurDir => continue,
                    Component::Normal(name) => path.push(name),
                    _ => return Err(instruction_error("path", relative)),
                }
                selected.insert(path.clone());
            }
        }
        for (path, previous) in &self.instruction_directory_identities {
            let checked = kuru_platform::fs::Directory::open(
                path,
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .map_err(|_| instruction_error("path", path))?;
            ensure!(
                checked
                    .is_within(&root)
                    .map_err(|_| instruction_error("path", path))?
                    && *previous == checked.identity().to_bytes(),
                "nested instruction directory changed after capture"
            );
        }
        let mut ordered = selected.iter().cloned().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then_with(|| left.cmp(right))
        });
        let mut cache = self.instruction_cache.clone();
        let capture = capture_instruction_sources(&self.workspace, &ordered, &mut cache)?;
        let mut active_directories = BTreeSet::new();
        for source in &capture.sources {
            let parent = source
                .path
                .parent()
                .ok_or_else(|| instruction_error("path", &source.path))?;
            let checked = kuru_platform::fs::Directory::open(
                parent,
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .map_err(|_| instruction_error("path", parent))?;
            ensure!(
                checked.identity().to_bytes() == source.directory_identity,
                "instruction source directory changed after capture"
            );
            if source.kind != InstructionSourceKind::Import
                && parent.starts_with(&self.workspace)
                && parent != self.workspace
            {
                let relative = parent
                    .strip_prefix(&self.workspace)
                    .map_err(|_| instruction_error("path", parent))?;
                let mut path = self.workspace.clone();
                for component in relative.components() {
                    let Component::Normal(name) = component else {
                        return Err(instruction_error("path", parent));
                    };
                    path.push(name);
                    active_directories.insert(path.clone());
                }
            }
        }
        let mut directory_identities = BTreeMap::new();
        for path in &active_directories {
            let checked = kuru_platform::fs::Directory::open(
                path,
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .map_err(|_| instruction_error("path", path))?;
            ensure!(
                checked
                    .is_within(&root)
                    .map_err(|_| instruction_error("path", path))?,
                "nested instruction directory escapes the workspace"
            );
            directory_identities.insert(path.clone(), checked.identity().to_bytes());
        }
        let active_paths = capture
            .sources
            .iter()
            .map(|source| source.path.clone())
            .collect::<BTreeSet<_>>();
        cache.retain(|path, _| {
            self.base_instruction_cache_paths.contains(path) || active_paths.contains(path)
        });
        let (value, origins) = self.value_with_preferences(&ProjectPreferences::default())?;
        let config: Config = value
            .try_into()
            .map_err(|_| config_error("type", &self.workspace))?;
        let manifest = derive_manifest(
            &config,
            &origins,
            &capture.sources,
            &self.prompt_catalog,
            &self.selected_material,
        )?;
        let new_authority = manifest.full_digest() != self.manifest.full_digest();
        let mut extended = self.clone();
        extended.manifest = manifest;
        extended.instruction_body = capture.rendered;
        extended.instructions = render_prompt_instructions(
            &extended.instruction_body,
            &self.prompt_catalog,
            &self.selected_material,
        );
        extended.instruction_notices = capture.notices;
        extended.instruction_sources = capture.sources;
        extended.instruction_cache = cache;
        extended.instruction_directories = active_directories;
        extended.instruction_directory_identities = directory_identities;
        Ok((extended, new_authority))
    }

    /// Recheck the captured nested directory objects before publishing a
    /// foreground activation. This does not reread or replace captured source
    /// bytes: later invocations perform their own fresh capture.
    pub fn revalidate_nested_directories(&self) -> Result<()> {
        let root = kuru_platform::fs::Directory::open(
            &self.workspace,
            kuru_platform::fs::Privacy::Inherited,
            kuru_platform::fs::NameRetention::Pinned,
        )
        .map_err(|_| instruction_error("path", &self.workspace))?;
        for (path, previous) in &self.instruction_directory_identities {
            let checked = kuru_platform::fs::Directory::open(
                path,
                kuru_platform::fs::Privacy::Inherited,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .map_err(|_| instruction_error("path", path))?;
            ensure!(
                checked
                    .is_within(&root)
                    .map_err(|_| instruction_error("path", path))?
                    && checked.identity().to_bytes() == *previous,
                "nested instruction directory changed after capture"
            );
        }
        Ok(())
    }

    /// Canonical source-set key for exact nested approval lookup. The complete
    /// manifest separately binds source bytes, identities and effective config.
    pub fn nested_instruction_source_paths(&self) -> Vec<[u8; 32]> {
        self.instruction_sources
            .iter()
            .filter(|source| !self.base_instruction_paths.contains(&source.path))
            .map(|source| source.path_digest)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn prompt_catalog(&self) -> &PromptCatalog {
        &self.prompt_catalog
    }

    /// The complete path-qualified supplemental source set, shared by nested
    /// instructions and selected skill material under the existing v2 record.
    pub fn supplemental_prompt_source_paths(&self) -> Vec<[u8; 32]> {
        let mut paths = self.nested_instruction_source_paths();
        paths.extend(
            self.selected_material
                .iter()
                .filter(|source| source.source.project)
                .map(|source| source_digest(source.source.path.as_os_str().as_encoded_bytes())),
        );
        paths.sort_unstable();
        paths.dedup();
        paths
    }

    /// Capture one selected skill body and, optionally, one direct reference.
    /// The returned snapshot is immutable and must be reviewed before use.
    pub fn with_selected_skill(&self, name: &str, reference: Option<&str>) -> Result<(Self, bool)> {
        let skill = self
            .prompt_catalog
            .skill(name)
            .ok_or_else(|| anyhow::anyhow!("skill is not in the effective catalog"))?;
        let additions = select_material(
            skill,
            reference,
            &self.workspace,
            self.user_prompt_root.as_deref(),
        )?;
        let mut selected = self.selected_material.clone();
        for addition in additions {
            if let Some(previous) = selected
                .iter()
                .find(|source| source.source.path == addition.source.path)
            {
                ensure!(
                    previous.content == addition.content
                        && previous.source.directory_identity == addition.source.directory_identity
                        && previous.source.file_identity == addition.source.file_identity,
                    "selected skill source changed during this invocation"
                );
            } else {
                selected.push(addition);
            }
        }
        selected.sort_by(|left, right| left.source.path.cmp(&right.source.path));
        ensure!(
            selected.len() <= MAX_MATERIAL_SOURCES
                && selected
                    .iter()
                    .map(|source| source.content.len())
                    .sum::<usize>()
                    <= MAX_MATERIAL_TOTAL,
            "selected skill material exceeds the prompt-source limit"
        );
        let (value, origins) = self.value_with_preferences(&ProjectPreferences::default())?;
        let config: Config = value
            .try_into()
            .map_err(|_| config_error("type", &self.workspace))?;
        let manifest = derive_manifest(
            &config,
            &origins,
            &self.instruction_sources,
            &self.prompt_catalog,
            &selected,
        )?;
        let changed = manifest.full_digest() != self.manifest.full_digest()
            || selected.len() != self.selected_material.len();
        let mut extended = self.clone();
        extended.manifest = manifest;
        extended.instructions =
            render_prompt_instructions(&self.instruction_body, &self.prompt_catalog, &selected);
        extended.selected_material = selected;
        Ok((extended, changed))
    }

    pub fn revalidate_selected_skill_sources(&self) -> Result<()> {
        for source in &self.selected_material {
            crate::prompt_sources::revalidate_material(
                &source.source,
                &self.workspace,
                self.user_prompt_root.as_deref(),
            )?;
        }
        Ok(())
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
        self.check_constraints(&config, true)?;
        Ok(config)
    }

    fn check_constraints(&self, config: &Config, include_saved_choices: bool) -> Result<()> {
        let final_value = toml::Value::try_from(config)?;
        for (path, expected) in &self.constraints {
            if !include_saved_choices
                && path.len() == 1
                && matches!(path[0].as_str(), "mode" | "model" | "effort")
            {
                continue;
            }
            let actual = lookup_path(&final_value, path);
            ensure!(
                actual == Some(expected),
                "managed constraint conflicts with final configuration at {}",
                safe_text(&path.join("."), 128)
            );
        }
        Ok(())
    }

    fn value_with_preferences(
        &self,
        preferences: &ProjectPreferences,
    ) -> Result<(toml::Value, BTreeMap<String, LayerOrigin>)> {
        let mut value = self.merged.clone();
        let mut origins = self.origins.clone();
        let mut typed = empty_table();
        for assignment in &self.overrides.typed_config {
            merge(&mut typed, typed_assignment(assignment)?);
        }
        let provider = self
            .overrides
            .provider
            .as_deref()
            .or_else(|| typed.get("provider")?.as_str())
            .or_else(|| self.local.as_ref()?.get("provider")?.as_str())
            .or_else(|| self.discovered_local.as_ref()?.get("provider")?.as_str())
            .or_else(|| value.get("provider")?.as_str())
            .ok_or_else(|| config_error("type", self.workspace()))?
            .to_owned();
        let explicit_model = self
            .overrides
            .model
            .as_deref()
            .or_else(|| typed.get("model")?.as_str())
            .or_else(|| self.local.as_ref()?.get("model")?.as_str())
            .or_else(|| self.discovered_local.as_ref()?.get("model")?.as_str())
            .map(str::to_owned);
        preferences
            .overlay(&mut value, &provider, explicit_model.as_deref())
            .map_err(|_| config_error("validation", self.workspace()))?;
        if let Some(local) = &self.discovered_local {
            merge_with_origins(
                &mut value,
                &mut origins,
                local.clone(),
                LayerOrigin {
                    source: SafeSource("project-local configuration".into()),
                    source_digest: source_digest(b"project-local configuration"),
                    automatic: false,
                },
            );
            origins.extend(self.discovered_local_origins.clone());
        }
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
        for assignment in &self.overrides.typed_config {
            let patch = typed_assignment(assignment)?;
            merge_with_origins(&mut value, &mut origins, patch, command_line_origin());
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
        self.hooks.validate()?;
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
        if let Some(reserve) = self.context_output_reserve_tokens {
            ensure!(
                (1..=2_000_000).contains(&reserve),
                "context_output_reserve_tokens must be between 1 and 2000000"
            );
        }
        ensure!(
            (50..=95).contains(&self.context_compaction_threshold_percent),
            "context_compaction_threshold_percent must be between 50 and 95"
        );
        ensure!(
            (1..=MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS)
                .contains(&self.context_compaction_output_reserve_tokens),
            "context_compaction_output_reserve_tokens must be between 1 and 2000000"
        );
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
            ensure!(
                mcp.allow_tools.len() <= 128 && mcp.deny_tools.len() <= 128,
                "MCP tool filters are limited to 128 allow and 128 deny globs"
            );
            for pattern in mcp.allow_tools.iter().chain(&mcp.deny_tools) {
                validate_mcp_tool_glob(pattern)?;
            }
            ensure!(
                mcp.header_env.len() <= 32,
                "HTTP MCP static headers are limited to 32 entries"
            );
            ensure!(
                mcp.command.is_none() || mcp.header_env.is_empty(),
                "stdio MCP servers cannot specify HTTP headers"
            );
            for (header, environment) in &mcp.header_env {
                validate_mcp_header_name(header)?;
                environment_name("MCP header environment reference", environment)?;
            }
            if let Some(oauth) = &mcp.oauth {
                ensure!(
                    mcp.command.is_none(),
                    "stdio MCP servers cannot enable OAuth"
                );
                validate_mcp_oauth(oauth)?;
                if oauth.enabled {
                    let url = Url::parse(
                        mcp.url
                            .as_deref()
                            .context("OAuth MCP server requires an HTTPS URL")?,
                    )
                    .context("OAuth MCP server URL is invalid")?;
                    ensure!(
                        url.scheme() == "https",
                        "OAuth MCP server URL must use HTTPS"
                    );
                    ensure!(
                        !mcp.header_env.keys().any(|header| matches!(
                            header.to_ascii_lowercase().as_str(),
                            "authorization" | "proxy-authorization"
                        )),
                        "OAuth MCP servers cannot configure static authorization headers"
                    );
                }
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

fn validate_mcp_tool_glob(pattern: &str) -> Result<()> {
    ensure!(
        !pattern.is_empty()
            && pattern.chars().count() <= 256
            && !pattern.chars().any(char::is_control)
            && !pattern
                .chars()
                .any(|character| matches!(character, '[' | ']' | '{' | '}')),
        "MCP tool globs must be 1–256 non-control characters and support only * and ? wildcards"
    );
    Ok(())
}

fn mcp_tool_glob_matches(pattern: &str, name: &str) -> bool {
    let name = name.chars().collect::<Vec<_>>();
    let mut previous = vec![false; name.len() + 1];
    previous[0] = true;
    for pattern_character in pattern.chars() {
        let mut next = vec![false; previous.len()];
        if pattern_character == '*' {
            next[0] = previous[0];
        }
        for index in 1..next.len() {
            next[index] = match pattern_character {
                '*' => previous[index] || next[index - 1],
                '?' => previous[index - 1],
                ordinary => previous[index - 1] && ordinary == name[index - 1],
            };
        }
        previous = next;
    }
    previous[name.len()]
}

fn validate_mcp_header_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte) }),
        "MCP static header names must be valid bounded HTTP field names"
    );
    ensure!(
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "accept"
                | "connection"
                | "content-length"
                | "content-type"
                | "host"
                | "mcp-protocol-version"
                | "mcp-session-id"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
        ),
        "MCP static header name is reserved for HTTP or MCP protocol ownership"
    );
    Ok(())
}

fn validate_mcp_oauth(oauth: &McpOAuthConfig) -> Result<()> {
    ensure!(
        !(oauth.client_id.is_some() && oauth.client_metadata_url.is_some()),
        "MCP OAuth configured client and client metadata identity are mutually exclusive"
    );
    ensure!(
        oauth.client_secret_env.is_none() || oauth.client_id.is_some(),
        "MCP OAuth client secret environment reference requires a configured client"
    );
    if let Some(client_id) = &oauth.client_id {
        nonempty("MCP OAuth client_id", client_id, 2048)?;
        ensure!(
            !client_id.chars().any(char::is_control),
            "MCP OAuth client_id contains control characters"
        );
    }
    if let Some(environment) = &oauth.client_secret_env {
        environment_name("MCP OAuth client secret environment reference", environment)?;
    }
    if let Some(metadata_url) = &oauth.client_metadata_url {
        endpoint("MCP OAuth client metadata URL", metadata_url)?;
        let parsed =
            Url::parse(metadata_url).context("MCP OAuth client metadata URL is invalid")?;
        ensure!(
            parsed.scheme() == "https",
            "MCP OAuth client metadata URL must use HTTPS"
        );
    }
    ensure!(
        oauth.scopes.len() <= 64,
        "MCP OAuth scope allowlist is limited to 64 entries"
    );
    let mut scopes = BTreeSet::new();
    for scope in &oauth.scopes {
        ensure!(
            !scope.is_empty()
                && scope.len() <= 256
                && scope.bytes().all(|byte| {
                    byte == b'!' || (b'#'..=b'[').contains(&byte) || (b']'..=b'~').contains(&byte)
                }),
            "MCP OAuth scopes must be bounded OAuth scope-token values"
        );
        ensure!(
            scopes.insert(scope),
            "MCP OAuth scope allowlist contains a duplicate"
        );
    }
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedDocument {
    defaults: Option<toml::Value>,
    constraints: Option<toml::Value>,
}

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn validate_patch(patch: &toml::Value, path: &Path) -> Result<()> {
    ensure!(patch.is_table(), "managed configuration requires tables");
    reject_removed_settings(patch).map_err(|_| config_error("validation", path))?;
    let mut checked = toml::Value::try_from(Config::default())?;
    merge(&mut checked, patch.clone());
    let _: Config = checked
        .try_into()
        .map_err(|error| config_type_error(path, &error))?;
    Ok(())
}

fn collect_constraints(
    value: &toml::Value,
    prefix: &[String],
    constraints: &mut Vec<(Vec<String>, toml::Value)>,
) {
    if let toml::Value::Table(table) = value
        && !(prefix.len() >= 2 && prefix[0] == "mcp")
        && !(!prefix.is_empty() && table.is_empty())
    {
        for (key, value) in table {
            let mut path = prefix.to_vec();
            path.push(key.clone());
            collect_constraints(value, &path, constraints);
        }
    } else if !prefix.is_empty() {
        constraints.push((prefix.to_vec(), value.clone()));
    }
}

fn lookup_path<'a>(value: &'a toml::Value, path: &[String]) -> Option<&'a toml::Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

fn parse_captured_patch(
    captured: Option<(&Path, &str)>,
    base: &toml::Value,
    total_bytes: &mut usize,
) -> Result<(Option<toml::Value>, BTreeMap<String, LayerOrigin>)> {
    let Some((path, source)) = captured else {
        return Ok((None, BTreeMap::new()));
    };
    ensure!(
        source.len() <= MAX_FILE_BYTES,
        "configuration input exceeds 256 KiB"
    );
    *total_bytes = total_bytes
        .checked_add(source.len())
        .ok_or_else(|| config_error("read", path))?;
    ensure!(
        *total_bytes <= MAX_COMBINED_BYTES,
        "configuration input exceeds 1 MiB"
    );
    let patch: toml::Value =
        toml::from_str(source).map_err(|error| config_parse_error(path, source, &error))?;
    reject_removed_settings(&patch).map_err(|_| config_error("validation", path))?;
    let mut checked = base.clone();
    merge(&mut checked, patch.clone());
    let _: Config = checked
        .try_into()
        .map_err(|error| config_type_error(path, &error))?;
    let mut origins = BTreeMap::new();
    record_origins(&patch, "", layer_origin(path, false), &mut origins);
    Ok((Some(patch), origins))
}

fn typed_assignment(assignment: &str) -> Result<toml::Value> {
    let (key, value) = assignment
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("-c expects key=value"))?;
    let key = key.trim();
    ensure!(
        !key.is_empty()
            && key.len() <= 256
            && key.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 64
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            })
            && !value.contains(['\r', '\n']),
        "-c has an invalid key or multiline value"
    );
    toml::from_str(&format!("{key}={value}"))
        .map_err(|_| anyhow::anyhow!("-c has an invalid TOML value"))
}

fn command_line_origin() -> LayerOrigin {
    LayerOrigin {
        source: SafeSource("command line".into()),
        source_digest: source_digest(b"command line"),
        automatic: false,
    }
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

/// The documented forward-compatibility policy, kept verbatim with
/// `docs/configuration.md` and the published configuration reference so a
/// rejection tells the user why a newer key is not silently ignored.
const UNKNOWN_KEY_POLICY: &str = "unknown keys are rejected: a configuration that needs a new key requires a newer Kuru version and never silently changes authority";

/// An unknown key is a forward-compatibility rejection, not an ordinary type
/// error; the offending key itself stays out of the message like every other
/// configuration diagnostic.
fn config_type_error(path: &Path, error: &toml::de::Error) -> anyhow::Error {
    if error.to_string().contains("unknown field") {
        anyhow::anyhow!(
            "configuration key error in {}: {UNKNOWN_KEY_POLICY}",
            safe_source(path)
        )
    } else {
        config_error("type", path)
    }
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
    let cli = command_line_origin;
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
        origins.insert("dream_every".into(), cli());
        origins.insert("dream_on_exit".into(), cli());
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
    prompt_catalog: &PromptCatalog,
    selected_material: &[PromptMaterial],
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
    let hook_origins = automatic_origins(origins, "hooks");
    if !config.hooks.is_empty() && !hook_origins.is_empty() {
        let hook_count = [
            HookEvent::PreTurn,
            HookEvent::PostTurn,
            HookEvent::PreTool,
            HookEvent::PostTool,
            HookEvent::SpeakerSelected,
        ]
        .into_iter()
        .map(|event| config.hooks.event(event).len())
        .sum::<usize>();
        push_claim(
            &mut claims,
            AuthorityClaimCategory::LifecycleHooks,
            &config.hooks,
            &hook_origins,
            format!("{hook_count} ordered lifecycle hook command(s)"),
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
        if !mcp.enabled {
            continue;
        }
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
                    (
                        name,
                        &mcp.command,
                        &mcp.args,
                        &mcp.env,
                        &mcp.allow_tools,
                        &mcp.deny_tools,
                    ),
                    &mcp_origins,
                    format!("{display_name}: configured executable"),
                )?;
            } else {
                push_claim(
                    &mut claims,
                    category,
                    (
                        name,
                        &mcp.url,
                        &mcp.allow_tools,
                        &mcp.deny_tools,
                        &mcp.header_env,
                        &mcp.oauth,
                    ),
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
                directory_identity: &source.directory_identity,
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
            .map(|source| {
                source_digest_with_identity(
                    source.path_digest,
                    source.directory_identity,
                    source.identity,
                )
            })
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
    let project_skills = prompt_catalog.project_skills().collect::<Vec<_>>();
    if !project_skills.is_empty() {
        let entries = project_skills
            .iter()
            .map(|skill| {
                (
                    &skill.name,
                    &skill.description,
                    source_digest(skill.source.path.as_os_str().as_encoded_bytes()),
                    skill.source.directory_identity,
                    skill.source.file_identity,
                    &skill.frontmatter,
                )
            })
            .collect::<Vec<_>>();
        push_prompt_sources_claim(
            &mut claims,
            AuthorityClaimCategory::ProjectSkillMetadata,
            &entries,
            project_skills.iter().map(|skill| &skill.source.path),
            project_skills.len(),
        )?;
    }
    let project_commands = prompt_catalog.project_commands().collect::<Vec<_>>();
    if !project_commands.is_empty() {
        let entries = project_commands
            .iter()
            .map(|command| {
                (
                    &command.name,
                    &command.description,
                    source_digest(command.source.path.as_os_str().as_encoded_bytes()),
                    command.source.directory_identity,
                    command.source.file_identity,
                    &command.content,
                )
            })
            .collect::<Vec<_>>();
        push_prompt_sources_claim(
            &mut claims,
            AuthorityClaimCategory::ProjectCommands,
            &entries,
            project_commands.iter().map(|command| &command.source.path),
            project_commands.len(),
        )?;
    }
    let project_material = selected_material
        .iter()
        .filter(|source| source.source.project)
        .collect::<Vec<_>>();
    if !project_material.is_empty() {
        let entries = project_material
            .iter()
            .map(|material| {
                (
                    material.kind,
                    &material.name,
                    source_digest(material.source.path.as_os_str().as_encoded_bytes()),
                    material.source.directory_identity,
                    material.source.file_identity,
                    &material.content,
                )
            })
            .collect::<Vec<_>>();
        push_prompt_sources_claim(
            &mut claims,
            AuthorityClaimCategory::ProjectSkillMaterial,
            &entries,
            project_material.iter().map(|source| &source.source.path),
            project_material.len(),
        )?;
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

fn push_prompt_sources_claim<'a>(
    claims: &mut Vec<AuthorityClaim>,
    category: AuthorityClaimCategory,
    entries: &impl Serialize,
    paths: impl Iterator<Item = &'a PathBuf>,
    count: usize,
) -> Result<()> {
    let paths = paths.collect::<Vec<_>>();
    let sources = paths
        .iter()
        .map(|path| safe_source(path))
        .collect::<Vec<_>>();
    let source_digests = paths
        .iter()
        .map(|path| source_digest(path.as_os_str().as_encoded_bytes()))
        .collect();
    claims.push(AuthorityClaim {
        category,
        digest: claim_digest(category, entries)?,
        source: sources
            .first()
            .expect("nonempty project prompt source claim")
            .clone(),
        sources,
        source_digests,
        display: SafeClaimDisplay(format!(
            "{count} effective source{}",
            if count == 1 { "" } else { "s" }
        )),
    });
    Ok(())
}

fn render_prompt_instructions(
    instructions: &str,
    catalog: &PromptCatalog,
    material: &[PromptMaterial],
) -> String {
    let mut rendered = instructions.to_owned();
    if catalog.skills().next().is_some() {
        rendered.push_str("\nAvailable skills (name and description only). Use skill_load to select a skill before following its body. Skill metadata and bodies never grant tools:\n");
        for skill in catalog.skills() {
            rendered.push_str("- ");
            rendered.push_str(&skill.name);
            rendered.push_str(": ");
            rendered.push_str(&serde_json::to_string(&skill.description).expect("string encodes"));
            rendered.push('\n');
        }
    }
    for source in material {
        rendered.push_str("\n--- selected ");
        rendered.push_str(match source.kind {
            MaterialKind::SkillBody => "skill body",
            MaterialKind::SkillReference => "skill reference",
        });
        rendered.push_str(": ");
        rendered.push_str(&safe_source(&source.source.path).to_string());
        rendered.push_str(" ---\n");
        rendered.push_str(&source.content);
        if !source.content.ends_with('\n') {
            rendered.push('\n');
        }
    }
    rendered
}

fn source_digest_with_identity(
    path_digest: [u8; 32],
    directory_identity: [u8; 24],
    identity: [u8; 24],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"kuru.workspace-trust.instruction-source\0");
    hash.update(path_digest);
    hash.update(directory_identity);
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

struct InstructionCapture {
    sources: Vec<InstructionSource>,
    rendered: String,
    notices: Vec<String>,
    seen_paths: BTreeSet<PathBuf>,
    seen_identities: BTreeSet<[u8; 24]>,
    active_paths: Vec<PathBuf>,
    active_identities: Vec<[u8; 24]>,
    total_bytes: usize,
    additional_omissions: usize,
}

#[derive(Clone, Debug)]
struct CheckedInstruction {
    content: Option<String>,
    directory_identity: [u8; 24],
    identity: [u8; 24],
}

impl InstructionCapture {
    fn new() -> Self {
        Self {
            sources: Vec::new(),
            rendered: String::new(),
            notices: Vec::new(),
            seen_paths: BTreeSet::new(),
            seen_identities: BTreeSet::new(),
            active_paths: Vec::new(),
            active_identities: Vec::new(),
            total_bytes: 0,
            additional_omissions: 0,
        }
    }

    fn notice(&mut self, path: &Path, reason: &str) {
        if self.notices.len() < MAX_INSTRUCTION_NOTICES {
            let message = format!(
                "Kuru omitted instruction source {}: {reason}",
                safe_source(path)
            );
            self.rendered.push_str(&format!("\n[{message}]\n"));
            self.notices.push(message);
        } else {
            self.additional_omissions += 1;
        }
    }

    fn visit(
        &mut self,
        path: PathBuf,
        scope: &Path,
        kind: InstructionSourceKind,
        depth: usize,
        optional: bool,
        cache: &mut BTreeMap<PathBuf, Option<CheckedInstruction>>,
    ) -> Result<()> {
        if optional {
            match cache.get(&path) {
                Some(None) => return Ok(()),
                Some(Some(_)) => {}
                None => match fs::symlink_metadata(&path) {
                    Ok(_) => {}
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        cache.insert(path, None);
                        return Ok(());
                    }
                    Err(_) => return Err(instruction_error("read", &path)),
                },
            }
        }
        if self.active_paths.contains(&path) {
            self.notice(&path, "import cycle");
            return Ok(());
        }
        if self.seen_paths.contains(&path) {
            return Ok(());
        }
        if depth > MAX_IMPORT_DEPTH {
            self.notice(&path, "eight-edge import depth limit");
            return Ok(());
        }
        if self.seen_paths.len() >= MAX_INSTRUCTION_PATHS {
            self.notice(&path, "128-source graph limit");
            return Ok(());
        }
        let checked = if let Some(cached) = cache.get(&path) {
            cached.clone()
        } else {
            let checked = read_checked_instruction(&path, optional)?;
            cache.insert(path.clone(), checked.clone());
            checked
        };
        let Some(CheckedInstruction {
            content,
            directory_identity,
            identity,
        }) = checked
        else {
            return Ok(());
        };
        if self.active_identities.contains(&identity) {
            self.notice(&path, "import cycle");
            return Ok(());
        }
        self.seen_paths.insert(path.clone());
        if !self.seen_identities.insert(identity) {
            return Ok(());
        }
        let Some(content) = content else {
            self.notice(&path, "256 KiB file limit");
            return Ok(());
        };
        if self.total_bytes.saturating_add(content.len()) > MAX_COMBINED_BYTES {
            self.notice(&path, "1 MiB combined instruction limit");
            return Ok(());
        }
        self.total_bytes += content.len();
        self.sources.push(InstructionSource {
            path: path.clone(),
            kind,
            path_digest: source_digest(path.as_os_str().as_encoded_bytes()),
            safe_source: safe_source(&path),
            directory_identity,
            identity,
            content: content.clone(),
        });
        self.active_paths.push(path.clone());
        self.active_identities.push(identity);
        self.rendered.push_str(&format!(
            "\n--- {}: {} ---\n",
            kind.prompt_name(),
            safe_source(&path)
        ));
        let mut fence: Option<(char, usize)> = None;
        for line in content.split_inclusive('\n') {
            let trimmed = line.trim();
            if let Some((marker, opening_length)) = fence {
                self.rendered.push_str(line);
                if instruction_fence(line).is_some_and(|(closing, length, suffix)| {
                    closing == marker
                        && length >= opening_length
                        && suffix
                            .bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                }) {
                    fence = None;
                }
                continue;
            }
            if let Some((marker, length, suffix)) = instruction_fence(line)
                && (marker != '`' || !suffix.contains('`'))
            {
                fence = Some((marker, length));
                self.rendered.push_str(line);
                continue;
            }
            if let Some(import) = instruction_import(trimmed) {
                let imported = resolve_instruction_import(&path, scope, import)?;
                let before = self.rendered.len();
                self.visit(
                    imported,
                    scope,
                    InstructionSourceKind::Import,
                    depth + 1,
                    false,
                    cache,
                )?;
                if self.rendered.len() != before {
                    self.rendered.push_str(&format!(
                        "\n--- resume {}: {} ---\n",
                        kind.prompt_name(),
                        safe_source(&path)
                    ));
                }
            } else {
                self.rendered.push_str(line);
            }
        }
        self.active_paths.pop();
        self.active_identities.pop();
        Ok(())
    }

    fn finish(mut self) -> Self {
        if self.additional_omissions > 0 {
            let message = format!(
                "Kuru omitted {} additional instruction sources or branches after the notice limit",
                self.additional_omissions
            );
            self.rendered.push_str(&format!("\n[{message}]\n"));
            self.notices.push(message);
        }
        if !self.sources.is_empty() || !self.notices.is_empty() {
            self.rendered = format!(
                "Project instructions follow from outermost to most local. Where instructions conflict, the most local applicable source takes precedence; higher-priority conversation instructions still apply.\n{}",
                self.rendered
            );
        }
        self
    }
}

fn capture_instruction_sources(
    project: &Path,
    nested_directories: &[PathBuf],
    cache: &mut BTreeMap<PathBuf, Option<CheckedInstruction>>,
) -> Result<InstructionCapture> {
    let mut capture = InstructionCapture::new();
    for directory in ancestor_directories(project)? {
        capture.visit(
            directory.join("AGENTS.md"),
            &directory,
            InstructionSourceKind::Agents,
            0,
            true,
            cache,
        )?;
        capture.visit(
            directory.join("CLAUDE.md"),
            &directory,
            InstructionSourceKind::Claude,
            0,
            true,
            cache,
        )?;
    }
    for directory in nested_directories {
        capture.visit(
            directory.join("AGENTS.md"),
            directory,
            InstructionSourceKind::Agents,
            0,
            true,
            cache,
        )?;
        capture.visit(
            directory.join("CLAUDE.md"),
            directory,
            InstructionSourceKind::Claude,
            0,
            true,
            cache,
        )?;
    }
    Ok(capture.finish())
}

fn read_checked_instruction(path: &Path, optional: bool) -> Result<Option<CheckedInstruction>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound && optional => return Ok(None),
        Err(_) => return Err(instruction_error("read", path)),
    }
    let parent = path
        .parent()
        .ok_or_else(|| instruction_error("path", path))?;
    let directory = kuru_platform::fs::Directory::open(
        parent,
        kuru_platform::fs::Privacy::Inherited,
        kuru_platform::fs::NameRetention::Pinned,
    )
    .map_err(|_| instruction_error("path", path))?;
    let name = path
        .file_name()
        .ok_or_else(|| instruction_error("path", path))?;
    let mut file = directory
        .read(name)
        .map_err(|_| instruction_error("read", path))?;
    let identity = kuru_platform::fs::regular_file_info(&file)
        .map_err(|_| instruction_error("read", path))?
        .identity
        .to_bytes();
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| instruction_error("read", path))?;
    directory
        .verify(name, &file)
        .map_err(|_| instruction_error("changed", path))?;
    let content = if bytes.len() > MAX_FILE_BYTES {
        None
    } else {
        Some(String::from_utf8(bytes).map_err(|_| instruction_error("encoding", path))?)
    };
    Ok(Some(CheckedInstruction {
        content,
        directory_identity: directory.identity().to_bytes(),
        identity,
    }))
}

fn instruction_import(line: &str) -> Option<&str> {
    line.strip_prefix('@')
        .filter(|path| path.ends_with(".md") && !path.is_empty())
}

fn instruction_fence(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let text = &line[indent..];
    let marker = text.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = text
        .bytes()
        .take_while(|byte| *byte == marker as u8)
        .count();
    (length >= 3).then_some((marker, length, &text[length..]))
}

fn resolve_instruction_import(source: &Path, scope: &Path, import: &str) -> Result<PathBuf> {
    let raw = Path::new(import);
    if raw.is_absolute() {
        return Err(instruction_error("path", source));
    }
    let mut target = source
        .parent()
        .ok_or_else(|| instruction_error("path", source))?
        .to_path_buf();
    for component in raw.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => target.push(name),
            Component::ParentDir if target != scope => {
                target.pop();
            }
            _ => return Err(instruction_error("path", source)),
        }
    }
    if !target.starts_with(scope) || target == scope {
        return Err(instruction_error("path", source));
    }
    Ok(target)
}

fn instruction_error(category: &str, path: &Path) -> anyhow::Error {
    anyhow::anyhow!("instruction {category} error in {}", safe_source(path))
}

/// Compose captured ancestor instructions and checked imports without review.
/// Application launches use `ConfigSnapshot` and its workspace trust preflight.
pub fn load_instructions(project: &Path) -> Result<String> {
    capture_instruction_sources(project, &[], &mut BTreeMap::new()).map(|capture| capture.rendered)
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
