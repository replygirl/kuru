#[cfg(all(windows, test))]
use std::io::Write;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::shell_diagnostic::ProjectedShellDiagnostic;
#[cfg(unix)]
use crate::shell_diagnostic::{ShellCapture, ShellFailureCategory, failure as shell_failure};
#[cfg(windows)]
use crate::shell_diagnostic::{
    ShellCapture, ShellFailureCategory, failure as shell_failure,
    failure_with_cleanup as shell_failure_with_cleanup,
};
#[cfg(unix)]
use crate::unix_shell::ShellRegistry;
use anyhow::{Context, Result, bail, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, sinks::UTF8};
use ignore::{WalkBuilder, overrides::OverrideBuilder};
use kuru_core::{Config, NativeTool, PermissionSelector, ProjectRelativeTarget, ToolSpec};
#[cfg(windows)]
use kuru_platform::fs::FileIdentity;
use kuru_platform::fs::{Directory, NameRetention, Privacy};
#[cfg(windows)]
use kuru_platform::fs::{regular_file_info, validate_component};
use serde_json::{Value, json};
#[cfg(all(unix, test))]
use std::process::Stdio;
use tokio::io::AsyncReadExt;
#[cfg(all(unix, test))]
use tokio::process::Command;
#[cfg(any(windows, test))]
use tokio::time::timeout;
#[cfg(windows)]
type PathGuard = Directory;
#[cfg(unix)]
type PathGuard = ();

#[cfg(unix)]
const UNIX_SHELL_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "NO_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
];

const MAX_SEARCH_FILES: usize = 10_000;
const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEARCH_MATCHES: usize = 2_000;
const MAX_SEARCH_LINE_BYTES: usize = 8 * 1024;
const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PAGE_LINES: usize = 100_000;

/// Instruction scope follows the directory containing a file. Listing a
/// directory acts on that directory itself; checked file targets never cause
/// capture to probe a synthetic `file/AGENTS.md` path.
fn instruction_directory(
    target: &ProjectRelativeTarget,
    directory_target: bool,
) -> Result<ProjectRelativeTarget> {
    if directory_target || target.as_str() == "." {
        return Ok(target.clone());
    }
    let parent = target
        .as_str()
        .rsplit_once('/')
        .map_or(".", |(parent, _)| parent);
    ProjectRelativeTarget::parse(parent.to_owned())
}

#[cfg(windows)]
const WINDOWS_SHELL_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "USERNAME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_ARCHITEW6432",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "NO_COLOR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "PATHEXT",
];

#[cfg(unix)]
fn unix_shell_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let environment: Vec<_> = environment.into_iter().collect();
    UNIX_SHELL_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            environment
                .iter()
                .find(|(key, _)| key == OsStr::new(name))
                .map(|(_, value)| (OsString::from(name), value.clone()))
        })
        .collect()
}

#[cfg(windows)]
fn windows_shell_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
    system_directory: &Path,
) -> Result<Vec<(OsString, OsString)>> {
    use kuru_platform::windows::process::environment_key_eq;

    let environment: Vec<_> = environment.into_iter().collect();
    let is_allowed = |key: &OsStr| {
        WINDOWS_SHELL_ENVIRONMENT
            .iter()
            .any(|name| environment_key_eq(key, OsStr::new(name)))
    };
    for (index, (key, _)) in environment
        .iter()
        .enumerate()
        .filter(|(_, (key, _))| is_allowed(key))
    {
        ensure!(
            !environment[index + 1..]
                .iter()
                .any(|(other, _)| is_allowed(other) && environment_key_eq(key, other)),
            "ambiguous case-equivalent Windows environment key"
        );
    }
    let mut projected: Vec<_> = WINDOWS_SHELL_ENVIRONMENT
        .iter()
        .filter_map(|name| {
            environment
                .iter()
                .find(|(key, _)| environment_key_eq(key, OsStr::new(name)))
                .map(|(_, value)| (OsString::from(name), value.clone()))
        })
        .collect();
    if !projected
        .iter()
        .any(|(key, _)| environment_key_eq(key, OsStr::new("PATHEXT")))
    {
        projected.push(("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()));
    }
    let windows = system_directory
        .parent()
        .context("native system directory lacks Windows parent")?;
    projected.extend([
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("WINDIR".into(), windows.as_os_str().into()),
        ("ComSpec".into(), system_directory.join("cmd.exe").into()),
    ]);
    Ok(projected)
}

use crate::{
    MAX_BYTES,
    file_edits::{CheckpointStore, CheckpointSummary, EditHunk, FileEffect, apply_hunks},
    instruction_review::{
        InstructionGate, InstructionGateOutcome, InstructionReviewSender, SkillGate,
    },
    mcp::{McpCatalog, McpExecution, McpHosts, McpStatus},
    mcp_cache::McpCatalogStore,
    permissions::{ApprovalSender, PermissionInvocation, PermissionOutcome, PermissionService},
    redaction,
    tool_output::{ProjectedToolError, ToolContent, ToolExecution, ToolFailure, ToolFailureKind},
    web_fetch,
};

mod parallel;
#[cfg(any(test, feature = "test-support"))]
pub use parallel::ParallelReadTestGate;
pub use parallel::{ParallelReadAdmission, ParallelReadCancellation, PreparedRead};

/// File tools operate under an opened directory capability. Shell and MCP
/// authorization grant process/server authority; cwd is not an OS sandbox.
pub struct ToolHost {
    root: PathBuf,
    directory: Dir,
    root_guard: Arc<Directory>,
    permissions: Arc<PermissionService>,
    checkpoints: Option<Arc<CheckpointStore>>,
    instruction_gate: Option<Arc<dyn InstructionGate>>,
    skill_gate: Option<Arc<dyn SkillGate>>,
    has_skills: bool,
    parallel_read_handles: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(any(test, feature = "test-support"))]
    parallel_read_barrier: Option<Arc<tokio::sync::Barrier>>,
    #[cfg(any(test, feature = "test-support"))]
    parallel_read_test_gate: Option<ParallelReadTestGate>,
    #[cfg(any(test, feature = "test-support"))]
    web_fetch_test_route: Option<(String, std::net::SocketAddr)>,
    mcp: McpHosts,
    #[cfg(unix)]
    shells: ShellRegistry,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolCatalog {
    tools: Vec<ToolSpec>,
    mcp: Vec<McpStatus>,
}

/// One actor call and an optional reviewed prompt update. The runtime settles
/// the original call once and installs the update before its next inference.
pub struct ActorToolOutcome {
    pub result: Result<String>,
    pub instructions: Option<String>,
    pub replan_required: bool,
}

/// The concrete provider call that proposed a tool effect. Direct user file
/// commands use a separate one-shot origin and never fabricate these fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocationContext {
    pub session_id: String,
    pub turn_id: String,
    pub actor_id: String,
    pub invocation_id: String,
    pub call_id: String,
}

impl ToolInvocationContext {
    fn receipt_id(&self) -> Result<String> {
        for field in [
            &self.session_id,
            &self.turn_id,
            &self.actor_id,
            &self.invocation_id,
            &self.call_id,
        ] {
            ensure!(
                !field.is_empty() && field.len() <= 256,
                "tool invocation provenance is incomplete or oversized"
            );
        }
        let encoded = serde_json::to_vec(&(
            "kuru-file-effect-v1",
            &self.session_id,
            &self.turn_id,
            &self.actor_id,
            &self.invocation_id,
            &self.call_id,
        ))?;
        Ok(format!("file-{}", crate::file_edits::hash(&encoded)))
    }
}

#[derive(Clone, Copy)]
enum ToolInvocationOrigin<'a> {
    DirectUser,
    Actor(&'a ToolInvocationContext),
    ActorUnattributed,
}

impl ToolCatalog {
    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    pub fn mcp(&self) -> &[McpStatus] {
        &self.mcp
    }

    pub fn into_tools(self) -> Vec<ToolSpec> {
        self.tools
    }
}

impl ToolHost {
    pub fn new(root: &Path, config: &Config) -> Result<Self> {
        let root = root.canonicalize().context("tool root does not exist")?;
        let root_guard = Arc::new(Directory::open(
            &root,
            Privacy::Inherited,
            NameRetention::Movable,
        )?);
        Self::with_retained_root(root_guard, config)
    }

    /// Construct a host from the workspace capability retained before
    /// configuration review. Configured process launches keep this exact guard.
    pub fn with_retained_root(root_guard: Arc<Directory>, config: &Config) -> Result<Self> {
        let permissions = Arc::new(PermissionService::unattended(
            root_guard.as_ref(),
            config.clone(),
        )?);
        Self::with_permission_service(root_guard, config, permissions)
    }

    /// Construct a host using the app-preflighted permission service. The
    /// service must bind this exact retained workspace and effective routes.
    pub fn with_permission_service(
        root_guard: Arc<Directory>,
        config: &Config,
        permissions: Arc<PermissionService>,
    ) -> Result<Self> {
        root_guard.revalidate()?;
        permissions.validate_context(root_guard.as_ref(), config)?;
        let root = root_guard.path().to_path_buf();
        let directory = Dir::open_ambient_dir(&root, ambient_authority())?;
        root_guard.revalidate()?;
        Ok(Self {
            mcp: McpHosts::with_retained_root(root_guard.clone(), &config.mcp)?,
            root,
            directory,
            root_guard,
            permissions,
            checkpoints: None,
            instruction_gate: None,
            skill_gate: None,
            has_skills: false,
            parallel_read_handles: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(any(test, feature = "test-support"))]
            parallel_read_barrier: None,
            #[cfg(any(test, feature = "test-support"))]
            parallel_read_test_gate: None,
            #[cfg(any(test, feature = "test-support"))]
            web_fetch_test_route: None,
            #[cfg(unix)]
            shells: ShellRegistry::new(),
        })
    }

    pub fn with_instruction_gate(mut self, gate: Arc<dyn InstructionGate>) -> Self {
        self.instruction_gate = Some(gate);
        self
    }

    /// Attach the app's checked private file checkpoint store. A mutating file
    /// tool refuses to run when this store is unavailable.
    pub fn with_checkpoint_store(mut self, store: Arc<CheckpointStore>) -> Result<Self> {
        store.validate_root(&self.root_guard)?;
        self.checkpoints = Some(store);
        Ok(self)
    }

    /// Attach app-owned private discovery metadata. Cached entries can inform
    /// catalogs but never install an executable MCP route.
    pub fn with_mcp_catalog_store(self, store: Arc<McpCatalogStore>) -> Result<Self> {
        self.mcp.install_cache(store)?;
        Ok(self)
    }

    pub fn with_skill_gate(mut self, gate: Arc<dyn SkillGate>, has_skills: bool) -> Self {
        self.skill_gate = Some(gate);
        self.has_skills = has_skills;
        self
    }

    /// Test-only execution barrier for proving that two checked native reads
    /// are simultaneously live. Install it before moving the host to runtime.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_parallel_read_barrier_for_test(
        mut self,
        barrier: Arc<tokio::sync::Barrier>,
    ) -> Self {
        self.parallel_read_barrier = Some(barrier);
        self
    }

    /// Test-only two-phase gate for cancellation and cleanup assertions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_parallel_read_test_gate(mut self, gate: ParallelReadTestGate) -> Self {
        self.parallel_read_test_gate = Some(gate);
        self
    }

    /// Current and maximum retained native handles available to prepared
    /// reads. Tests use this to prove partial admission releases its permits.
    #[cfg(any(test, feature = "test-support"))]
    pub fn parallel_read_handle_usage_for_test(&self) -> (usize, usize) {
        (
            self.parallel_read_handles
                .load(std::sync::atomic::Ordering::Acquire),
            parallel::MAX_PREPARED_READ_HANDLES,
        )
    }

    /// Test-only exact resolver route for exercising the production fetch
    /// transport against one isolated local server.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_web_fetch_test_route(
        mut self,
        host: impl Into<String>,
        address: std::net::SocketAddr,
    ) -> Self {
        self.web_fetch_test_route = Some((host.into(), address));
        self
    }

    /// Verify the held workspace identity without reopening a replacement.
    pub fn revalidate_root(&self) -> Result<()> {
        Ok(self.root_guard.revalidate()?)
    }

    /// Confirm that a runtime configuration still matches this host's retained
    /// workspace and immutable permission service before it dispatches tools.
    pub fn validate_permission_context(&self, config: &Config) -> Result<()> {
        self.permissions
            .validate_context(self.root_guard.as_ref(), config)
    }

    /// Shared with the runtime for outbound A2A admission. The service itself
    /// retains no UI channel; callers lend one only for a foreground operation.
    pub fn permission_service(&self) -> Arc<PermissionService> {
        Arc::clone(&self.permissions)
    }

    pub fn inspect_file_checkpoint(&self, id: &str) -> Result<Option<CheckpointSummary>> {
        self.checkpoints
            .as_ref()
            .context("private file checkpoint store is unavailable")?
            .inspect(id)
    }

    pub fn list_file_checkpoints(&self, limit: usize) -> Result<Vec<CheckpointSummary>> {
        self.checkpoints
            .as_ref()
            .context("private file checkpoint store is unavailable")?
            .list(limit)
    }

    pub fn prune_file_checkpoint(&self, id: &str, discard_uncertain: bool) -> Result<bool> {
        self.checkpoints
            .as_ref()
            .context("private file checkpoint store is unavailable")?
            .prune(id, discard_uncertain)
    }

    /// Explicit user action; this is deliberately absent from the model tool
    /// catalog. Undo is newly authorized for the exact target before effects.
    pub async fn undo_file_checkpoint(
        &self,
        id: &str,
        approval: Option<&ApprovalSender>,
    ) -> Result<CheckpointSummary> {
        let store = self
            .checkpoints
            .as_ref()
            .context("private file checkpoint store is unavailable")?;
        let selected = store
            .inspect(id)?
            .context("selected file checkpoint does not exist")?;
        let target = self.validated_permission_target(&selected.path, true)?;
        let selector = PermissionSelector::native(if selected.created {
            NativeTool::FileDelete
        } else {
            NativeTool::FileWrite
        });
        let undo_args = json!({"path": selected.path.as_str()});
        let invocation = PermissionInvocation::new(selector, Some(target.clone()), &undo_args)?;
        ensure!(
            self.permissions
                .authorize(&invocation, approval)
                .await?
                .is_authorized(),
            "file undo requires exact target permission"
        );
        self.root_guard.revalidate()?;
        let now = store
            .inspect(id)?
            .context("selected file checkpoint disappeared")?;
        ensure!(
            now.id == selected.id
                && now.path == selected.path
                && now.effect == selected.effect
                && now.state == selected.state
                && now.created == selected.created,
            "selected file checkpoint changed during permission review"
        );
        ensure!(
            self.validated_permission_target(&now.path, true)? == target,
            "file undo target changed during permission review"
        );
        #[cfg(windows)]
        let (_, final_name, path_guard) = self.path(&now.path, true)?;
        #[cfg(unix)]
        let (_, final_name, _) = self.path(&now.path, true)?;
        #[cfg(windows)]
        let parent_identity = path_guard.identity();
        #[cfg(windows)]
        drop(path_guard);
        let parent_path = self
            .root
            .join(Path::new(&now.path).parent().unwrap_or(Path::new(".")));
        #[cfg(windows)]
        let parent =
            reopen_movable_parent(&self.root_guard, &parent_path, parent_identity, "file undo")?;
        #[cfg(unix)]
        let parent = Directory::open(&parent_path, Privacy::Inherited, NameRetention::Movable)?;
        ensure!(
            parent.is_within(&self.root_guard)?,
            "file undo parent changed outside project root"
        );
        store.lease()?.undo(id, &parent, final_name.as_os_str())
    }

    /// Admit one configured outbound A2A call using the same immutable service
    /// as native/MCP tools. The runtime has no platform dependency and never
    /// owns grants or a process-wide approval sender.
    pub async fn authorize_a2a(
        &self,
        alias: &str,
        endpoint: &str,
        arguments: &Value,
        approval: Option<&ApprovalSender>,
    ) -> Result<()> {
        self.permissions.validate_a2a_route(alias, endpoint)?;
        let selector = PermissionSelector::a2a(alias)?;
        let invocation = PermissionInvocation::new(selector, None, arguments)?;
        match self.permissions.authorize(&invocation, approval).await? {
            outcome if outcome.is_authorized() => {
                self.root_guard.revalidate()?;
                Ok(())
            }
            PermissionOutcome::Denied => Err(ProjectedToolError::new(
                ToolFailureKind::PermissionDenied,
                String::new(),
            )
            .into()),
            PermissionOutcome::PermissionRequired => Err(ProjectedToolError::new(
                ToolFailureKind::PermissionRequired,
                String::new(),
            )
            .into()),
            _ => unreachable!("permission service returned an invalid outcome"),
        }
    }

    /// Canonical pathname paired with the retained root capability.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn specs(&self) -> Result<Vec<ToolSpec>> {
        Ok(self.catalog().await?.into_tools())
    }

    pub async fn catalog(&self) -> Result<ToolCatalog> {
        let mut specs = Vec::new();
        if self.has_skills {
            let mut skill = spec(
                "skill_load",
                "Select one catalog skill and optionally one direct Markdown reference. Selection reviews project prompt authority; it grants no tool permissions or script execution.",
                &["name"],
                &["name"],
            );
            skill.parameters["properties"]["reference"] = json!({"type":"string"});
            specs.push(skill);
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::FileRead))
        {
            let mut file_read = spec(
                "file_read",
                "Read a UTF-8 project file with a bounded head-and-tail excerpt, or request one-based logical-line pages.",
                &["path"],
                &["path"],
            );
            file_read.parameters["properties"]["offset"] =
                json!({"type":"integer","minimum":1,"default":1});
            file_read.parameters["properties"]["limit"] =
                json!({"type":"integer","minimum":1,"maximum":10000,"default":200});
            specs.push(file_read);
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::FileList))
        {
            specs.push(spec(
                "file_list",
                "List immediate children of a project directory. Protected paths are omitted.",
                &["path"],
                &[],
            ));
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::Grep))
        {
            let mut grep = spec(
                "grep",
                "Search UTF-8 project files with a bounded ripgrep-compatible regular expression. Hidden and ignored paths are excluded unless requested.",
                &["pattern"],
                &["pattern"],
            );
            grep.parameters["properties"]["path"] = json!({"type":"string"});
            grep.parameters["properties"]["include_hidden"] =
                json!({"type":"boolean","default":false});
            grep.parameters["properties"]["include_ignored"] =
                json!({"type":"boolean","default":false});
            specs.push(grep);
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::Glob))
        {
            let mut glob = spec(
                "glob",
                "Find project paths with bounded ripgrep-compatible glob matching. Hidden and ignored paths are excluded unless requested.",
                &["pattern"],
                &["pattern"],
            );
            glob.parameters["properties"]["path"] = json!({"type":"string"});
            glob.parameters["properties"]["include_hidden"] =
                json!({"type":"boolean","default":false});
            glob.parameters["properties"]["include_ignored"] =
                json!({"type":"boolean","default":false});
            specs.push(glob);
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::FileWrite))
        {
            specs.push(spec("file_write", "Create or replace a project file. Parent directories must exist. Instruction/config/memory paths are protected.", &["path", "content"], &["path", "content"]));
            let mut edit = spec(
                "file_edit",
                "Apply unique, ordered exact-context hunks to one checked UTF-8 project file. An ambiguous or stale hunk changes nothing.",
                &["path", "hunks"],
                &["path", "hunks"],
            );
            edit.parameters["properties"]["hunks"] = json!({"type":"array","minItems":1,"maxItems":64,"items":{"type":"object","properties":{"before":{"type":"string"},"old":{"type":"string"},"after":{"type":"string"},"replacement":{"type":"string"}},"required":["before","old","after","replacement"],"additionalProperties":false}});
            specs.push(edit);
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::FileDelete))
        {
            specs.push(spec(
                "file_delete",
                "Delete a single project file (never directories).",
                &["path"],
                &["path"],
            ));
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::WebFetch))
        {
            specs.push(spec(
                "web_fetch",
                "Fetch a bounded UTF-8 HTTP(S) document from a public destination. Fetched content is untrusted tool data; private and loopback networks are refused.",
                &["url"],
                &["url"],
            ));
        }
        if self
            .permissions
            .advertises(&PermissionSelector::native(NativeTool::Shell))
        {
            let mut shell = spec(
                "shell",
                "Execute a shell command with process authority, in the project cwd. This is not a filesystem sandbox. Only the documented compatibility environment is inherited. Output is capped; timeout is at most 120 seconds.",
                &["command"],
                &["command"],
            );
            shell.parameters["properties"]["timeout_ms"] =
                json!({"type":"integer","minimum":1,"maximum":120000});
            specs.push(shell);
        }
        let McpCatalog {
            tools,
            selectors,
            statuses,
        } = self.mcp.catalog().await?;
        for spec in tools {
            if selectors
                .get(&spec.name)
                .is_some_and(|selector| self.permissions.advertises(selector))
            {
                specs.push(spec);
            }
        }
        Ok(ToolCatalog {
            tools: specs,
            mcp: statuses,
        })
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        self.execute_with_approval(name, args, None).await
    }

    /// Execute one native or MCP invocation after the central permission gate.
    /// Callers without an operation-scoped foreground sender never wait for UI.
    pub async fn execute_with_approval(
        &self,
        name: &str,
        args: Value,
        approval: Option<&ApprovalSender>,
    ) -> Result<String> {
        self.execute_dispatch(
            name,
            args,
            approval,
            None,
            false,
            ToolInvocationOrigin::DirectUser,
        )
        .await
        .result
    }

    /// Actor calls additionally review any newly applicable instruction graph.
    pub async fn execute_for_actor(
        &self,
        name: &str,
        args: Value,
        permission_approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
    ) -> ActorToolOutcome {
        self.execute_dispatch(
            name,
            args,
            permission_approval,
            instruction_approval,
            true,
            ToolInvocationOrigin::ActorUnattributed,
        )
        .await
    }

    /// Runtime-authorized call with the actual admitted turn and provider
    /// invocation identities. Actor file effects fail closed without them.
    pub async fn execute_for_actor_with_context(
        &self,
        name: &str,
        args: Value,
        permission_approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
        context: &ToolInvocationContext,
    ) -> ActorToolOutcome {
        self.execute_dispatch(
            name,
            args,
            permission_approval,
            instruction_approval,
            true,
            ToolInvocationOrigin::Actor(context),
        )
        .await
    }

    async fn execute_dispatch(
        &self,
        name: &str,
        args: Value,
        approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
        actor: bool,
        origin: ToolInvocationOrigin<'_>,
    ) -> ActorToolOutcome {
        let mut instructions = None;
        let mut replan_required = false;
        if name == "skill_load" {
            let result: std::result::Result<ToolExecution, ToolFailure> = async {
                if !actor {
                    return Err(ToolFailure::built_in(anyhow::anyhow!(
                        "skill_load is actor-only"
                    )));
                }
                if !args.is_object() {
                    return Err(ToolFailure::built_in(anyhow::anyhow!(
                        "skill arguments must be an object"
                    )));
                }
                let selected = string(&args, "name").map_err(ToolFailure::built_in)?;
                let reference = args
                    .get("reference")
                    .map(|value| value.as_str().context("reference must be a string"))
                    .transpose()
                    .map_err(ToolFailure::built_in)?;
                let gate = self
                    .skill_gate
                    .as_ref()
                    .context("skill catalog is unavailable")
                    .map_err(ToolFailure::built_in)?;
                match gate
                    .review_skill(selected, reference, instruction_approval)
                    .await
                    .map_err(ToolFailure::built_in)?
                {
                    InstructionGateOutcome::Unchanged => {}
                    InstructionGateOutcome::Proposed(proposal) => {
                        self.root_guard
                            .revalidate()
                            .map_err(|error| ToolFailure::built_in(error.into()))?;
                        instructions =
                            Some(proposal.publish().await.map_err(ToolFailure::built_in)?);
                    }
                    InstructionGateOutcome::Denied => {
                        return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                            "skill prompt authority was denied"
                        )));
                    }
                    InstructionGateOutcome::Required(detail) => {
                        return Err(ToolFailure::permission_required(anyhow::anyhow!(detail)));
                    }
                }
                Ok(ToolExecution::ProjectedText(format!(
                    "Skill {selected} is active; continue with its reviewed instructions."
                )))
            }
            .await;
            return ActorToolOutcome {
                result: match result {
                    Ok(execution) => project_execution(execution),
                    Err(failure) => project_failure(failure),
                },
                instructions,
                replan_required,
            };
        }
        // Search discovers many independent file targets. It has no request
        // root grant: every returned candidate is authorized with the same
        // foreground sender and exact target before its path or contents can
        // enter the result.
        if matches!(name, "grep" | "glob") {
            let selector = PermissionSelector::native(if name == "grep" {
                NativeTool::Grep
            } else {
                NativeTool::Glob
            });
            if !self.permissions.advertises(&selector) {
                return ActorToolOutcome {
                    result: project_failure(ToolFailure::permission_denied(anyhow::anyhow!(
                        "tool permission was denied"
                    ))),
                    instructions,
                    replan_required,
                };
            }
            let result = match self
                .execute_inner(
                    name,
                    args,
                    approval,
                    instruction_approval,
                    actor,
                    origin,
                    None,
                    &mut instructions,
                )
                .await
            {
                Ok(execution) => project_execution(execution),
                Err(failure) => project_failure(failure),
            };
            return ActorToolOutcome {
                result,
                instructions,
                replan_required,
            };
        }
        let result: std::result::Result<ToolExecution, ToolFailure> = async {
            let (selector, target) = self.permission_facts(name, &args).await?;
            let invocation = PermissionInvocation::new(selector, target, &args)
                .map_err(ToolFailure::built_in)?;
            match self
                .permissions
                .authorize(&invocation, approval)
                .await
                .map_err(ToolFailure::built_in)?
            {
                outcome if outcome.is_authorized() => {
                    // Permission storage or a foreground answer may have waited.
                    // Re-derive the checked facts before dispatch so a changed
                    // path spelling, case alias, parent, or MCP route cannot
                    // turn this invocation into a different operation.
                    let (current_selector, current_target) =
                        self.permission_facts(name, &args).await?;
                    if current_selector != *invocation.selector()
                        || current_target.as_ref() != invocation.target()
                    {
                        return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                            "tool permission target changed while approval was pending"
                        )));
                    }
                    // The operation still has the same checked facts; confirm
                    // the reviewed workspace identity immediately before its
                    // effect, including an operation backed by a held file
                    // capability.
                    self.root_guard
                        .revalidate()
                        .map_err(|error| ToolFailure::built_in(error.into()))?;
                    if actor && let (Some(gate), Some(target)) =
                        (&self.instruction_gate, invocation.target())
                    {
                        let directory = instruction_directory(
                            target,
                            matches!(
                                invocation.selector(),
                                PermissionSelector::Native { name: NativeTool::FileList }
                            ),
                        )
                        .map_err(ToolFailure::built_in)?;
                        let proposed = match gate
                            .review(std::slice::from_ref(&directory), instruction_approval)
                            .await
                            .map_err(ToolFailure::built_in)?
                        {
                            InstructionGateOutcome::Unchanged => None,
                            InstructionGateOutcome::Proposed(proposal) => Some(proposal),
                            InstructionGateOutcome::Denied => {
                                return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                                    "nested instruction authority was denied"
                                )));
                            }
                            InstructionGateOutcome::Required(detail) => {
                                return Err(ToolFailure::permission_required(anyhow::anyhow!(detail)));
                            }
                        };
                        // A foreground answer can span a target replacement.
                        let (current_selector, current_target) =
                            self.permission_facts(name, &args).await?;
                        if current_selector != *invocation.selector()
                            || current_target.as_ref() != invocation.target()
                        {
                            return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                                "tool permission target changed while instruction review was pending"
                            )));
                        }
                        self.root_guard
                            .revalidate()
                            .map_err(|error| ToolFailure::built_in(error.into()))?;
                        if let Some(proposal) = proposed {
                            instructions = Some(proposal.publish().await.map_err(ToolFailure::built_in)?);
                            if matches!(
                                invocation.selector(),
                                PermissionSelector::Native {
                                    name: NativeTool::FileWrite | NativeTool::FileDelete
                                }
                            ) {
                                replan_required = true;
                                return Ok(ToolExecution::ProjectedText(
                                    "New path-specific instructions were activated. Replan this file change before executing it; the original call made no change.".into(),
                                ));
                            }
                        }
                    }
                }
                PermissionOutcome::Denied => {
                    return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                        "tool permission was denied"
                    )));
                }
                PermissionOutcome::PermissionRequired => {
                    return Err(ToolFailure::permission_required(anyhow::anyhow!(
                        "foreground permission approval is required"
                    )));
                }
                _ => unreachable!("permission service returned an invalid outcome"),
            }
            let authorized_target = invocation.target().cloned();
            self.execute_inner(name, args, approval, instruction_approval, actor, origin, authorized_target.as_ref(), &mut instructions).await
        }
        .await;
        let result = match result {
            Ok(execution) => project_execution(execution),
            Err(failure) => project_failure(failure),
        };
        ActorToolOutcome {
            result,
            instructions,
            replan_required,
        }
    }

    async fn permission_facts(
        &self,
        name: &str,
        args: &Value,
    ) -> std::result::Result<(PermissionSelector, Option<ProjectRelativeTarget>), ToolFailure> {
        let native = |tool| PermissionSelector::native(tool);
        let facts = match name {
            "file_read" => (
                native(NativeTool::FileRead),
                Some(
                    self.validated_permission_target(
                        string(args, "path").map_err(ToolFailure::built_in)?,
                        false,
                    )
                    .map_err(ToolFailure::built_in)?,
                ),
            ),
            "file_list" => {
                let path = args
                    .get("path")
                    .map(|value| value.as_str().context("path must be a string"))
                    .transpose()
                    .map_err(ToolFailure::built_in)?
                    .unwrap_or(".");
                (
                    native(NativeTool::FileList),
                    Some(
                        self.validated_permission_target(path, false)
                            .map_err(ToolFailure::built_in)?,
                    ),
                )
            }
            "file_write" | "file_edit" => (
                native(NativeTool::FileWrite),
                Some(
                    self.validated_permission_target(
                        string(args, "path").map_err(ToolFailure::built_in)?,
                        true,
                    )
                    .map_err(ToolFailure::built_in)?,
                ),
            ),
            "file_delete" => (
                native(NativeTool::FileDelete),
                Some(
                    self.validated_permission_target(
                        string(args, "path").map_err(ToolFailure::built_in)?,
                        true,
                    )
                    .map_err(ToolFailure::built_in)?,
                ),
            ),
            "shell" => (native(NativeTool::Shell), None),
            "web_fetch" => (native(NativeTool::WebFetch), None),
            _ => (
                self.mcp
                    .selector(name)
                    .await
                    .map_err(|failure| ToolFailure {
                        kind: failure.kind,
                        error: failure.error,
                    })?,
                None,
            ),
        };
        Ok(facts)
    }

    fn validated_permission_target(
        &self,
        value: &str,
        writing: bool,
    ) -> Result<ProjectRelativeTarget> {
        self.root_guard.revalidate()?;
        let _path_guard = self.path(value, writing)?;
        #[cfg(any(windows, target_os = "macos"))]
        return self.physical_permission_target(value);
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let path = Path::new(value);
            let mut components = Vec::new();
            for component in path.components() {
                match component {
                    Component::CurDir => {}
                    Component::Normal(name) => components
                        .push(name.to_str().context(
                            "tool path must use UTF-8 spelling for permission matching",
                        )?),
                    _ => anyhow::bail!("tool path cannot traverse outside the project root"),
                }
            }
            let spelling = if components.is_empty() {
                ".".into()
            } else {
                components.join("/")
            };
            ProjectRelativeTarget::parse(spelling)
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn physical_permission_target(&self, value: &str) -> Result<ProjectRelativeTarget> {
        // The physical spelling is the authorization target. This prevents a
        // case variant (and on Windows a DOS 8.3 alias) from bypassing a rule.
        let candidate = self.root.join(value);
        let expanded = match candidate.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let input = Path::new(value);
                let parent = input.parent().unwrap_or_else(|| Path::new("."));
                let final_name = input
                    .file_name()
                    .context("permission target lacks a final component")?;
                self.root.join(parent).canonicalize()?.join(final_name)
            }
            Err(error) => return Err(error.into()),
        };
        let root = self.root.canonicalize()?;
        let relative = expanded
            .strip_prefix(root)
            .context("expanded permission target is outside the retained project root")?;
        let mut components = Vec::new();
        for component in relative.components() {
            if let Component::Normal(name) = component {
                components.push(
                    name.to_str()
                        .context("expanded permission target is not valid UTF-8")?,
                );
            }
        }
        ProjectRelativeTarget::parse(if components.is_empty() {
            ".".into()
        } else {
            components.join("/")
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "native and MCP dispatch keeps approval, authority and result projection explicit"
    )]
    async fn execute_inner(
        &self,
        name: &str,
        args: Value,
        approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
        actor: bool,
        origin: ToolInvocationOrigin<'_>,
        authorized_target: Option<&ProjectRelativeTarget>,
        instructions: &mut Option<String>,
    ) -> std::result::Result<ToolExecution, ToolFailure> {
        if !args.is_object() {
            return Err(ToolFailure::built_in(anyhow::anyhow!(
                "tool arguments must be an object"
            )));
        }
        match name {
            "file_read" => {
                let execution = async {
                    let (directory, path, _guard) = self.path(string(&args, "path")?, false)?;
                    let mut options = OpenOptions::new();
                    options.read(true).follow(FollowSymlinks::No);
                    #[cfg(unix)]
                    {
                        use cap_std::fs::OpenOptionsExt;
                        options.custom_flags(nix::libc::O_NONBLOCK);
                    }
                    let file = directory.open_with(path, &options)?;
                    #[cfg(windows)]
                    let file = {
                        let file = file.into_std();
                        ensure!(
                            regular_file_info(&file)?.links == 1,
                            "hard-linked files are not readable through file tools"
                        );
                        file
                    };
                    ensure!(
                        file.metadata()?.is_file(),
                        "file_read requires a regular file"
                    );
                    #[cfg(unix)]
                    {
                        use cap_std::fs::MetadataExt;
                        ensure!(
                            file.metadata()?.nlink() == 1,
                            "hard-linked files are not readable through file tools"
                        );
                    }
                    // Bound the stream to the initially checked regular-file extent. A
                    // concurrent append cannot keep the scanner reading indefinitely.
                    let extent = file.metadata()?.len();
                    #[cfg(unix)]
                    let file = file.into_std();
                    if args.get("offset").is_some() || args.get("limit").is_some() {
                        return Ok(ToolExecution::Json(page_file_read(
                            file,
                            extent,
                            page_offset(&args)?,
                            page_limit(&args)?,
                        )?));
                    }
                    Ok(ToolExecution::ProjectedText(
                        read_file_output(tokio::fs::File::from_std(file), extent).await?,
                    ))
                }
                .await;
                execution.map_err(ToolFailure::built_in)
            }
            "glob" => self
                .glob(&args, approval, instruction_approval, actor, instructions)
                .await
                .map(ToolExecution::Json),
            "grep" => self
                .grep(&args, approval, instruction_approval, actor, instructions)
                .await
                .map(ToolExecution::Json),
            "file_write" | "file_edit" | "file_delete" => self
                .file_mutation(name, &args, origin, authorized_target)
                .map(ToolExecution::Text)
                .map_err(ToolFailure::built_in),
            "file_list" => {
                let execution = (|| -> Result<ToolExecution> {
                    let input_path = args
                        .get("path")
                        .map(|value| value.as_str().context("path must be a string"))
                        .transpose()?
                        .unwrap_or(".");
                    let (directory, path, _guard) = self.path(input_path, false)?;
                    #[cfg(windows)]
                    let _listing_guard = Directory::open(
                        &_guard.path().join(&path),
                        Privacy::Inherited,
                        NameRetention::Pinned,
                    )?;
                    let directory = directory.open_dir_nofollow(path)?;
                    let mut entries = BTreeMap::new();
                    for entry in directory.entries()? {
                        let entry = entry?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        if protected_component(&name, false) || entry.file_type()?.is_symlink() {
                            continue;
                        }
                        entries.insert(
                            name,
                            if entry.file_type()?.is_dir() {
                                "directory"
                            } else {
                                "file"
                            },
                        );
                        ensure!(
                            entries.len() <= 10_000,
                            "directory exceeds 10000 entries; select a narrower path"
                        );
                    }
                    Ok(ToolExecution::Json(serde_json::to_value(entries)?))
                })();
                execution.map_err(ToolFailure::built_in)
            }
            "web_fetch" => {
                let execution = async {
                    let url = string(&args, "url")?;
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some((host, address)) = &self.web_fetch_test_route {
                        return web_fetch::fetch_with_test_route(url, host, *address)
                            .await
                            .map(ToolExecution::Json);
                    }
                    web_fetch::fetch(url).await.map(ToolExecution::Json)
                }
                .await;
                execution.map_err(ToolFailure::built_in)
            }
            "shell" => {
                let execution = async {
                    let duration = args
                        .get("timeout_ms")
                        .map(|value| {
                            value
                                .as_u64()
                                .context("timeout_ms must be a positive integer")
                        })
                        .transpose()?
                        .unwrap_or(30_000);
                    ensure!(
                        (1..=120_000).contains(&duration),
                        "timeout_ms must be 1..120000"
                    );
                    #[cfg(unix)]
                    let result = shell(
                        self.shells(),
                        self.root_guard.clone(),
                        self.root.clone(),
                        string(&args, "command")?,
                        Duration::from_millis(duration),
                    )
                    .await?;
                    #[cfg(windows)]
                    let result = shell(
                        &self.root_guard,
                        &self.root,
                        string(&args, "command")?,
                        Duration::from_millis(duration),
                    )
                    .await?;
                    Ok(ToolExecution::ProjectedJson(
                        serde_json::from_str(&result).context("shell emitted invalid result")?,
                    ))
                }
                .await;
                execution.map_err(ToolFailure::built_in)
            }
            _ => match self.mcp.execute(name, args).await {
                Ok(McpExecution::Success(result)) => Ok(ToolExecution::Json(result)),
                Ok(McpExecution::ApplicationError(content)) => {
                    Ok(ToolExecution::ApplicationError {
                        kind: ToolFailureKind::McpApplication,
                        content: ToolContent::Json(content),
                    })
                }
                Err(failure) => Err(ToolFailure {
                    kind: failure.kind,
                    error: failure.error,
                }),
            },
        }
    }

    async fn glob(
        &self,
        args: &Value,
        approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
        actor: bool,
        instructions: &mut Option<String>,
    ) -> std::result::Result<Value, ToolFailure> {
        let pattern =
            bounded_search_pattern(string(args, "pattern").map_err(ToolFailure::built_in)?)
                .map_err(ToolFailure::built_in)?;
        let mut omitted = SearchOmissions::default();
        let targets = self
            .admitted_search_targets(
                NativeTool::Glob,
                args,
                Some(&pattern),
                approval,
                &mut omitted,
            )
            .await
            .map_err(ToolFailure::built_in)?;
        self.review_search_targets(&targets, instruction_approval, actor, instructions)
            .await?;
        let matches: Vec<_> = targets
            .iter()
            .map(|target| target.as_str().to_owned())
            .collect();
        Ok(json!({"matches": matches, "omitted": omitted.into_json()}))
    }

    async fn grep(
        &self,
        args: &Value,
        approval: Option<&ApprovalSender>,
        instruction_approval: Option<&InstructionReviewSender>,
        actor: bool,
        instructions: &mut Option<String>,
    ) -> std::result::Result<Value, ToolFailure> {
        let pattern =
            bounded_search_pattern(string(args, "pattern").map_err(ToolFailure::built_in)?)
                .map_err(ToolFailure::built_in)?;
        let matcher = RegexMatcher::new_line_matcher(&pattern)
            .context("grep pattern is not a valid line regular expression")
            .map_err(ToolFailure::built_in)?;
        let mut omitted = SearchOmissions::default();
        let targets = self
            .admitted_search_targets(NativeTool::Grep, args, None, approval, &mut omitted)
            .await
            .map_err(ToolFailure::built_in)?;
        self.review_search_targets(&targets, instruction_approval, actor, instructions)
            .await?;
        self.grep_targets(&matcher, targets, omitted)
            .map_err(ToolFailure::built_in)
    }

    fn grep_targets(
        &self,
        matcher: &RegexMatcher,
        targets: Vec<ProjectRelativeTarget>,
        mut omitted: SearchOmissions,
    ) -> Result<Value> {
        (|| -> Result<Value> {
            let mut matches = Vec::new();
            for target in targets {
                self.root_guard.revalidate()?;
                let (directory, path, _guard) = self.path(target.as_str(), false)?;
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let mut file = directory.open_with(path, &options)?;
                let metadata = file.metadata()?;
                if !metadata.is_file() || metadata.len() > MAX_SEARCH_FILE_BYTES {
                    omitted.large_or_non_file += 1;
                    continue;
                }
                let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
                Read::by_ref(&mut file)
                    .take(MAX_SEARCH_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_SEARCH_FILE_BYTES {
                    omitted.large_or_non_file += 1;
                    continue;
                }
                if std::str::from_utf8(&bytes).is_err() {
                    omitted.unreadable += 1;
                    continue;
                }
                let target_name = target.as_str().to_owned();
                let mut searcher = Searcher::new();
                searcher
                    .search_slice(matcher, &bytes, UTF8(|line, text| {
                        if text.len() > MAX_SEARCH_LINE_BYTES {
                            omitted.oversized_line += 1;
                            return Ok(true);
                        }
                        matches.push(json!({"path": target_name, "line": line, "text": text.trim_end_matches(['\n', '\r'])}));
                        Ok(matches.len() < MAX_SEARCH_MATCHES)
                    }))
                    .map_err(|error| anyhow::anyhow!("grep search failed: {error}"))?;
                if matches.len() == MAX_SEARCH_MATCHES {
                    omitted.output_limit = true;
                    break;
                }
            }
            Ok(json!({"matches": matches, "omitted": omitted.into_json()}))
        })()
    }

    async fn admitted_search_targets(
        &self,
        tool: NativeTool,
        args: &Value,
        pattern: Option<&str>,
        approval: Option<&ApprovalSender>,
        omitted: &mut SearchOmissions,
    ) -> Result<Vec<ProjectRelativeTarget>> {
        let mut allowed = Vec::new();
        for target in self.search_candidates(args, pattern, omitted)? {
            match self
                .authorize_search_candidate(tool, &target, args, approval)
                .await?
            {
                SearchCandidateAccess::Allowed => allowed.push(target),
                SearchCandidateAccess::Denied => omitted.denied += 1,
                SearchCandidateAccess::PermissionRequired => omitted.permission_required += 1,
            }
            if tool == NativeTool::Glob && allowed.len() == MAX_SEARCH_MATCHES {
                omitted.output_limit = true;
                break;
            }
        }
        Ok(allowed)
    }

    async fn review_search_targets(
        &self,
        targets: &[ProjectRelativeTarget],
        approval: Option<&InstructionReviewSender>,
        actor: bool,
        instructions: &mut Option<String>,
    ) -> std::result::Result<(), ToolFailure> {
        if !actor || targets.is_empty() {
            return Ok(());
        }
        let Some(gate) = &self.instruction_gate else {
            return Ok(());
        };
        let directories = targets
            .iter()
            .map(|target| instruction_directory(target, false))
            .collect::<Result<Vec<_>>>()
            .map_err(ToolFailure::built_in)?;
        let proposed = match gate
            .review(&directories, approval)
            .await
            .map_err(ToolFailure::built_in)?
        {
            InstructionGateOutcome::Unchanged => None,
            InstructionGateOutcome::Proposed(proposal) => Some(proposal),
            InstructionGateOutcome::Denied => {
                return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                    "nested instruction authority was denied"
                )));
            }
            InstructionGateOutcome::Required(detail) => {
                return Err(ToolFailure::permission_required(anyhow::anyhow!(detail)));
            }
        };
        // The foreground answer can span a link or parent replacement. Never
        // publish reviewed bytes from a path whose checked target has changed.
        for target in targets {
            let current = self
                .validated_permission_target(target.as_str(), false)
                .map_err(ToolFailure::built_in)?;
            if current != *target {
                return Err(ToolFailure::permission_denied(anyhow::anyhow!(
                    "search candidate changed while instruction review was pending"
                )));
            }
        }
        self.root_guard
            .revalidate()
            .map_err(|error| ToolFailure::built_in(error.into()))?;
        if let Some(proposal) = proposed {
            *instructions = Some(proposal.publish().await.map_err(ToolFailure::built_in)?);
        }
        Ok(())
    }

    fn search_candidates(
        &self,
        args: &Value,
        pattern: Option<&str>,
        omitted: &mut SearchOmissions,
    ) -> Result<Vec<ProjectRelativeTarget>> {
        let root = optional_path(args)?;
        self.root_guard.revalidate()?;
        let _path_guard = self.path(root, false)?;
        let include_hidden = optional_bool(args, "include_hidden")?;
        let include_ignored = optional_bool(args, "include_ignored")?;
        let mut walker = WalkBuilder::new(self.root.join(root));
        walker
            .follow_links(false)
            .hidden(!include_hidden)
            .ignore(!include_ignored)
            .git_global(!include_ignored)
            .git_ignore(!include_ignored)
            .git_exclude(!include_ignored)
            // A project need not itself be a Git checkout for its local
            // `.gitignore` policy to be useful to native search.
            .require_git(false)
            .threads(1);
        let overrides = if let Some(pattern) = pattern {
            let mut overrides = OverrideBuilder::new(&self.root);
            overrides.add(pattern).context("glob pattern is invalid")?;
            Some(overrides.build().context("glob pattern is invalid")?)
        } else {
            None
        };
        let mut candidates = Vec::new();
        for entry in walker.build() {
            let Ok(entry) = entry else {
                omitted.unreadable += 1;
                continue;
            };
            if !entry.file_type().is_some_and(|type_| type_.is_file()) {
                continue;
            }
            // Walk overrides take precedence over ignore rules. Match the glob
            // after traversal so a pattern never re-admits ignored files.
            if let Some(overrides) = &overrides
                && !overrides.matched(entry.path(), false).is_whitelist()
            {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&self.root) else {
                omitted.unreadable += 1;
                continue;
            };
            let Some(relative) = relative.to_str() else {
                omitted.unreadable += 1;
                continue;
            };
            let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
            // An explicit glob override can otherwise admit dot-prefixed paths
            // after the walker's hidden filter has rejected them.
            if !include_hidden
                && relative
                    .split('/')
                    .any(|component| component.starts_with('.'))
            {
                continue;
            }
            let Ok(target) = self.validated_permission_target(&relative, false) else {
                omitted.protected_or_linked += 1;
                continue;
            };
            candidates.push(target);
            if candidates.len() == MAX_SEARCH_FILES {
                omitted.scan_limit = true;
                break;
            }
        }
        Ok(candidates)
    }

    async fn authorize_search_candidate(
        &self,
        tool: NativeTool,
        target: &ProjectRelativeTarget,
        args: &Value,
        approval: Option<&ApprovalSender>,
    ) -> Result<SearchCandidateAccess> {
        let invocation = PermissionInvocation::new(
            PermissionSelector::native(tool),
            Some(target.clone()),
            args,
        )?;
        match self.permissions.authorize(&invocation, approval).await? {
            outcome if outcome.is_authorized() => {
                self.root_guard.revalidate()?;
                Ok(SearchCandidateAccess::Allowed)
            }
            PermissionOutcome::Denied => Ok(SearchCandidateAccess::Denied),
            PermissionOutcome::PermissionRequired => Ok(SearchCandidateAccess::PermissionRequired),
            _ => unreachable!("permission service returned an invalid outcome"),
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        #[cfg(unix)]
        {
            let (shell, mcp) = tokio::join!(self.shells.shutdown(), self.mcp.shutdown());
            match (shell, mcp) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(shell), Ok(())) => Err(shell),
                (Ok(()), Err(mcp)) => Err(mcp),
                (Err(shell), Err(mcp)) => {
                    Err(shell).context(format!("MCP shutdown also failed: {mcp:#}"))
                }
            }
        }
        #[cfg(not(unix))]
        self.mcp.shutdown().await
    }

    #[cfg(unix)]
    fn shells(&self) -> &ShellRegistry {
        &self.shells
    }

    #[cfg(all(unix, test))]
    fn test_shells(&self) -> &ShellRegistry {
        &self.shells
    }

    fn file_mutation(
        &self,
        name: &str,
        args: &Value,
        origin: ToolInvocationOrigin<'_>,
        authorized_target: Option<&ProjectRelativeTarget>,
    ) -> Result<String> {
        let value = string(args, "path")?;
        #[cfg(windows)]
        let (_, final_name, path_guard) = self.path(value, true)?;
        #[cfg(unix)]
        let (_, final_name, _) = self.path(value, true)?;
        let target = self.validated_permission_target(value, true)?;
        ensure!(
            authorized_target == Some(&target),
            "file permission target changed before mutation"
        );
        #[cfg(windows)]
        let parent_identity = path_guard.identity();
        #[cfg(windows)]
        drop(path_guard);
        let parent_path = self
            .root
            .join(Path::new(value).parent().unwrap_or(Path::new(".")));
        #[cfg(windows)]
        let parent =
            reopen_movable_parent(&self.root_guard, &parent_path, parent_identity, "file")?;
        #[cfg(unix)]
        let parent = Directory::open(&parent_path, Privacy::Inherited, NameRetention::Movable)?;
        ensure!(
            parent.is_within(&self.root_guard)?,
            "file parent changed outside project root"
        );
        let store = self
            .checkpoints
            .as_ref()
            .context("private file checkpoint store is unavailable")?;
        store.validate_root(&self.root_guard)?;
        let lease = store.lease()?;
        let id = match origin {
            ToolInvocationOrigin::DirectUser => uuid::Uuid::new_v4().to_string(),
            ToolInvocationOrigin::Actor(context) => context.receipt_id()?,
            ToolInvocationOrigin::ActorUnattributed => {
                bail!("actor file mutation lacks admitted provider invocation provenance")
            }
        };
        let fingerprint = crate::file_edits::hash(&serde_json::to_vec(&(name, args))?);
        let path = target.as_str();
        let summary = match name {
            "file_write" => {
                let content = string(args, "content")?;
                ensure!(
                    content.len() <= MAX_BYTES,
                    "file content exceeds 2 MiB limit"
                );
                lease.mutate(
                    &id,
                    &fingerprint,
                    path,
                    FileEffect::Write,
                    &parent,
                    final_name.as_os_str(),
                    |_| Ok(Some(content.as_bytes().to_vec())),
                )?
            }
            "file_edit" => {
                let hunks = args.get("hunks").context("file_edit requires hunks")?;
                let hunks: Vec<EditHunk> =
                    serde_json::from_value(hunks.clone()).context("invalid file_edit hunks")?;
                lease.mutate(
                    &id,
                    &fingerprint,
                    path,
                    FileEffect::Edit,
                    &parent,
                    final_name.as_os_str(),
                    |before| {
                        let before =
                            before.context("file_edit requires an existing regular file")?;
                        Ok(Some(apply_hunks(before, &hunks)?))
                    },
                )?
            }
            "file_delete" => lease.mutate(
                &id,
                &fingerprint,
                path,
                FileEffect::Delete,
                &parent,
                final_name.as_os_str(),
                |before| {
                    ensure!(
                        before.is_some(),
                        "file_delete requires an existing regular file"
                    );
                    Ok(None)
                },
            )?,
            _ => bail!("unknown file mutation"),
        };
        Ok(format!("{} completed; checkpoint {}", name, summary.id))
    }

    fn path(&self, value: &str, writing: bool) -> Result<(Dir, PathBuf, PathGuard)> {
        let path = Path::new(value);
        ensure!(
            !value.is_empty() && !path.is_absolute(),
            "tool path must be relative to the project root"
        );
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    #[cfg(windows)]
                    validate_component(name)?;
                    ensure!(
                        !protected_component(&name.to_string_lossy(), writing),
                        "protected instruction, config, credential, or memory path"
                    );
                    components.push(name);
                }
                _ => bail!("tool path cannot traverse outside the project root"),
            }
        }
        let mut directory = self.directory.try_clone()?;
        let final_name = PathBuf::from(components.pop().unwrap_or(std::ffi::OsStr::new(".")));
        #[cfg(windows)]
        let guard = {
            let parent = components
                .iter()
                .fold(self.root.clone(), |path, component| path.join(component));
            let guard = Directory::open(&parent, Privacy::Inherited, NameRetention::Pinned)?;
            ensure!(
                guard.is_within(&self.root_guard)?,
                "tool parent is outside the retained project root"
            );
            self.check_expanded_names(&parent, writing)?;
            guard
        };
        #[cfg(unix)]
        let guard = ();
        for component in components {
            directory = directory.open_dir_nofollow(component)?;
        }
        match directory.symlink_metadata(&final_name) {
            Ok(metadata) => {
                ensure!(
                    !metadata.is_symlink(),
                    "symlink paths are not permitted by file tools"
                );
                #[cfg(windows)]
                self.check_expanded_names(&guard.path().join(&final_name), writing)?;
                #[cfg(windows)]
                if metadata.is_file() {
                    // All reparse types and hardlinks are checked from the native
                    // handle, beyond cap-std's ordinary symlink classification.
                    drop(guard.read(final_name.as_os_str())?);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok((directory, final_name, guard))
    }

    #[cfg(windows)]
    fn check_expanded_names(&self, path: &Path, writing: bool) -> Result<()> {
        // Confinement is already established by held native parent identities.
        // Windows canonicalization expands DOS 8.3 names; inspect that spelling
        // too so AUTH~1.JSO cannot alias an otherwise protected auth.json.
        let expanded = path.canonicalize()?;
        let root = self.root.canonicalize()?;
        let relative = expanded
            .strip_prefix(&root)
            .context("expanded tool path is outside the project root")?;
        for component in relative.components() {
            if let Component::Normal(name) = component {
                ensure!(
                    !protected_component(&name.to_string_lossy(), writing),
                    "protected instruction, config, credential, or memory path"
                );
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct SearchOmissions {
    denied: usize,
    permission_required: usize,
    protected_or_linked: usize,
    unreadable: usize,
    large_or_non_file: usize,
    oversized_line: usize,
    scan_limit: bool,
    output_limit: bool,
}

impl SearchOmissions {
    fn into_json(self) -> Value {
        json!({
            "denied": self.denied,
            "permission_required": self.permission_required,
            "protected_or_linked": self.protected_or_linked,
            "unreadable": self.unreadable,
            "large_or_non_file": self.large_or_non_file,
            "oversized_line": self.oversized_line,
            "scan_limit_reached": self.scan_limit,
            "output_limit_reached": self.output_limit,
        })
    }
}

enum SearchCandidateAccess {
    Allowed,
    Denied,
    PermissionRequired,
}

fn project_execution(execution: ToolExecution) -> Result<String> {
    match execution {
        ToolExecution::Text(text) => project_text(text),
        ToolExecution::Json(value) => project_json(value),
        ToolExecution::ProjectedText(text) => Ok(text),
        ToolExecution::ProjectedJson(value) => Ok(serde_json::to_string(&value)?),
        ToolExecution::ApplicationError { kind, content } => {
            let detail = project_content(content)?;
            Err(ProjectedToolError::new(kind, detail).into())
        }
    }
}

fn project_failure(failure: ToolFailure) -> Result<String> {
    let detail = if let Some(diagnostic) = failure
        .error
        .downcast_ref::<ProjectedShellDiagnostic>()
        .map(|diagnostic| diagnostic.0.clone())
    {
        diagnostic
    } else {
        project_text(format!("{:#}", failure.error))?
    };
    Err(ProjectedToolError::new(failure.kind, detail).into())
}

fn project_content(content: ToolContent) -> Result<String> {
    match content {
        ToolContent::Json(value) => project_json(value),
    }
}

fn project_text(text: String) -> Result<String> {
    redaction::text(&text).map_err(|_| ProjectedToolError::output_withheld().into())
}

fn project_json(value: Value) -> Result<String> {
    redaction::json(value).map_err(|_| ProjectedToolError::output_withheld().into())
}

async fn read_file_output(mut file: tokio::fs::File, extent: u64) -> Result<String> {
    let mut output = redaction::StreamingProjection::new(MAX_BYTES);
    let mut pending = Vec::new();
    let mut buffer = [0; 8192];
    let mut remaining = extent;
    while remaining > 0 {
        let read = buffer.len().min(remaining.try_into().unwrap_or(usize::MAX));
        let count = file.read(&mut buffer[..read]).await?;
        if count == 0 {
            break;
        }
        remaining -= count as u64;
        pending.extend_from_slice(&buffer[..count]);
        match std::str::from_utf8(&pending) {
            Ok(_) => {
                output.push(&pending)?;
                pending.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                output.push(&pending[..valid])?;
                pending = pending[valid..].to_vec();
            }
            Err(_) => bail!("file is not UTF-8"),
        }
    }
    ensure!(pending.is_empty(), "file is not UTF-8");
    output.finish().map_err(Into::into)
}

#[cfg(windows)]
fn reopen_movable_parent(
    root_guard: &Directory,
    parent_path: &Path,
    expected_identity: FileIdentity,
    operation: &str,
) -> Result<Directory> {
    let parent = Directory::open(parent_path, Privacy::Inherited, NameRetention::Movable)?;
    ensure!(
        parent.identity() == expected_identity,
        "{operation} parent changed during handle handoff"
    );
    ensure!(
        parent.is_within(root_guard)?,
        "{operation} parent changed outside project root"
    );
    Ok(parent)
}

fn optional_path(args: &Value) -> Result<&str> {
    args.get("path")
        .map(|value| value.as_str().context("path must be a string"))
        .transpose()?
        .map_or(Ok("."), |path| {
            ensure!(!path.is_empty(), "path must not be empty");
            Ok(path)
        })
}

fn optional_bool(args: &Value, name: &str) -> Result<bool> {
    args.get(name)
        .map(|value| {
            value
                .as_bool()
                .with_context(|| format!("{name} must be a boolean"))
        })
        .transpose()
        .map(|value| value.unwrap_or(false))
}

fn bounded_search_pattern(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty() && value.chars().count() <= 512 && !value.chars().any(char::is_control),
        "search pattern must be 1–512 non-control characters"
    );
    Ok(value.into())
}

fn page_offset(args: &Value) -> Result<usize> {
    let value = args
        .get("offset")
        .map(|value| value.as_u64().context("offset must be a positive integer"))
        .transpose()?
        .unwrap_or(1);
    usize::try_from(value)
        .context("offset is too large")
        .and_then(|value| {
            ensure!(value > 0, "offset must be at least 1");
            Ok(value)
        })
}

fn page_limit(args: &Value) -> Result<usize> {
    let value = args
        .get("limit")
        .map(|value| value.as_u64().context("limit must be a positive integer"))
        .transpose()?
        .unwrap_or(200);
    usize::try_from(value)
        .context("limit is too large")
        .and_then(|value| {
            ensure!(
                (1..=10_000).contains(&value),
                "limit must be between 1 and 10000"
            );
            Ok(value)
        })
}

fn page_file_read(
    mut file: std::fs::File,
    extent: u64,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    ensure!(
        extent <= MAX_PAGE_BYTES,
        "paged file_read exceeds 2 MiB limit; select a smaller file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(extent).unwrap_or(0));
    Read::by_ref(&mut file)
        .take(MAX_PAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PAGE_BYTES,
        "paged file_read exceeds 2 MiB limit; select a smaller file"
    );
    let text = std::str::from_utf8(&bytes).context("file is not UTF-8")?;
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    ensure!(
        lines.len() <= MAX_PAGE_LINES,
        "paged file_read exceeds 100000 line limit"
    );
    ensure!(
        offset <= lines.len().saturating_add(1),
        "offset is beyond the end of the file"
    );
    let start = offset - 1;
    let end = start.saturating_add(limit).min(lines.len());
    let selected = lines[start..end].concat();
    Ok(json!({
        "text": selected,
        "offset": offset,
        "limit": limit,
        "line_count": lines.len(),
        "next_offset": (end < lines.len()).then_some(end + 1),
        "omitted_lines": lines.len().saturating_sub(end),
    }))
}

fn protected_component(value: &str, writing: bool) -> bool {
    let name = value.to_ascii_lowercase();
    matches!(
        name.as_str(),
        ".git"
            | ".kuru"
            | ".codex"
            | ".agents"
            | ".claude"
            | ".ssh"
            | ".aws"
            | ".azure"
            | ".gnupg"
            | ".mcp.json"
            | "auth.json"
            | "credentials.json"
            | "memory.sqlite"
            | "memory.db"
    ) || name == ".env"
        || name.starts_with(".env.")
        // A failed checked publication can leave its staged replacement in
        // the project directory. Keep its private bytes outside every native
        // file and search projection, including include_hidden searches.
        || name.starts_with(".kuru-edit-")
        || (writing
            && matches!(
                name.as_str(),
                "agents.md" | "claude.md" | "mise.toml" | "hk.pkl"
            ))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("{key} must be a string"))
}

fn spec(name: &str, description: &str, fields: &[&str], required: &[&str]) -> ToolSpec {
    let properties: BTreeMap<_, _> = fields
        .iter()
        .map(|name| (*name, json!({"type":"string"})))
        .collect();
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}

#[cfg(unix)]
async fn shell(
    registry: &ShellRegistry,
    root_guard: Arc<Directory>,
    root: PathBuf,
    command: &str,
    duration: Duration,
) -> Result<String> {
    let result = registry
        .execute(root_guard, root, command.into(), duration, || {
            unix_shell_environment(std::env::vars_os())
        })
        .await;
    match result {
        Err(error) if error.downcast_ref::<ProjectedShellDiagnostic>().is_some() => Err(error),
        Err(_) => Err(shell_failure(
            ShellFailureCategory::OperationFailed,
            &ShellCapture::new(),
        )),
        Ok(result) => Ok(result),
    }
}

#[cfg(windows)]
async fn shell(
    root_guard: &Directory,
    root: &Path,
    command: &str,
    duration: Duration,
) -> Result<String> {
    let result = shell_inner(root_guard, root, command, duration).await;
    match result {
        Err(error) if error.downcast_ref::<ProjectedShellDiagnostic>().is_some() => Err(error),
        Err(_) => Err(shell_failure(
            ShellFailureCategory::OperationFailed,
            &ShellCapture::new(),
        )),
        Ok(result) => Ok(result),
    }
}

#[cfg(windows)]
#[cfg(test)]
fn windows_shell_fixture_stage_path() -> Option<PathBuf> {
    if std::env::var("KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD").as_deref() != Ok("1") {
        return None;
    }
    let root = std::env::var_os("KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_ROOT")?;
    let case = match std::env::var("NO_COLOR").as_deref() {
        Ok("inherited") => "inherited",
        Ok("fallback") => "fallback",
        _ => return None,
    };
    Some(
        PathBuf::from(root)
            .join("temporary")
            .join(format!("shell-stage-{case}.txt")),
    )
}

#[cfg(windows)]
async fn shell_inner(
    root_guard: &Directory,
    root: &Path,
    command: &str,
    duration: Duration,
) -> Result<String> {
    use base64::Engine;
    use kuru_platform::windows::process::{
        Stdio, configured_command, system_directory, wait_process_handle,
    };
    ensure!(!command.trim().is_empty(), "shell command is empty");
    // This is deliberately authorized PowerShell source, not command argv.
    // Stock Windows PowerShell accepts UTF-16LE source; its actual exit status
    // is returned without a suffix that could accidentally replace `$?`.
    // Headless progress (including first-use module discovery) is not a text
    // diagnostic: Windows PowerShell 5.1 serializes it onto redirected stderr.
    // Disable only progress before any cmdlet runs; preserve warning/error and
    // literal stderr bytes, including text which happens to resemble CLIXML.
    // Load the two shipped modules used by Kuru's stock-shell contracts from
    // their exact PSHOME manifests. This retains arbitrary module autoloading
    // while avoiding generic cold command discovery for their first command.
    #[cfg(test)]
    let fixture_stage = windows_shell_fixture_stage_path();
    #[cfg(test)]
    let [before_imports, after_management, after_utility] = fixture_stage.as_ref().map_or_else(
        || [String::new(), String::new(), String::new()],
        |stage| {
            let stage = stage.to_string_lossy().replace('\'', "''");
            ["source-entered", "management-imported", "utility-imported"]
                .map(|name| format!("[IO.File]::AppendAllText('{stage}', \"{name}`n\");\n"))
        },
    );
    #[cfg(not(test))]
    let [before_imports, after_management, after_utility] = ["", "", ""];
    let source = format!(
        concat!(
            "{before_imports}",
            "$ProgressPreference = 'SilentlyContinue'; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding;\n",
            "$null = Microsoft.PowerShell.Core\\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\\Microsoft.PowerShell.Management\\Microsoft.PowerShell.Management.psd1')) -ErrorAction Stop;\n",
            "{after_management}",
            "$null = Microsoft.PowerShell.Core\\Import-Module -Name ([IO.Path]::Combine($PSHOME, 'Modules\\Microsoft.PowerShell.Utility\\Microsoft.PowerShell.Utility.psd1')) -ErrorAction Stop;\n",
            "{after_utility}",
            "{command}"
        ),
        before_imports = before_imports,
        after_management = after_management,
        after_utility = after_utility,
        command = command
    );
    let bytes: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut args: Vec<std::ffi::OsString> = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
    ]
    .map(Into::into)
    .into();
    args.push(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
    );
    // Use the same identity-checked launch spelling as other configured native
    // commands. PowerShell's .NET file APIs cannot use an introduced verbatim cwd.
    let root_pin = Directory::open(root, Privacy::Inherited, NameRetention::Pinned)?;
    let system_directory = system_directory()?;
    let program = system_directory.join("WindowsPowerShell/v1.0/powershell.exe");
    // This owned stock-shell launch must reconstruct its own module paths: a
    // PowerShell 7 parent can otherwise leave incompatible modules through Kuru.
    // Generic configured commands retain their caller's deliberate environment.
    let environment = windows_shell_environment(std::env::vars_os(), &system_directory)?;
    let mut spec = configured_command(program.as_os_str(), &args, root, environment)?;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    root_guard.revalidate()?;
    let mut child = spec
        .spawn()
        .await
        .context("cannot start Windows PowerShell")?;
    drop(root_pin);
    let mut stdout = child.take_stdout().context("missing shell stdout")?;
    let mut stderr = child.take_stderr().context("missing shell stderr")?;
    let mut out = ShellCapture::new();
    let mut err = ShellCapture::new();
    let operation = async {
        tokio::try_join!(out.read(&mut stdout), err.read(&mut stderr))?;
        out.finish()?;
        err.finish()?;
        let status = child.wait(duration).await?;
        Ok::<_, anyhow::Error>(status)
    };
    let result = match timeout(duration, operation).await {
        Err(_) => Err(ShellFailureCategory::TimedOut),
        Ok(Ok(status)) => Ok(status),
        Ok(Err(_)) => Err(ShellFailureCategory::CaptureFailed),
    };
    let mut cleanup_unconfirmed = false;
    let result = match result {
        Ok(status) => Ok(status),
        Err(category) => {
            // Query the retained root separately from whole-Job quiescence;
            // observation failure must never prevent the existing cleanup.
            let root_status = match child.duplicate_process_handle() {
                Ok(process) => wait_process_handle(&process, Duration::ZERO).await,
                Err(error) => Err(error),
            };
            #[cfg(test)]
            if let Some(stage) = &fixture_stage {
                let marker = match &root_status {
                    Ok(()) => "root-exited\n",
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => "root-alive\n",
                    Err(_) => "root-status-unknown\n",
                };
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(stage)
                    .and_then(|mut file| file.write_all(marker.as_bytes()));
            }
            #[cfg(not(test))]
            let _ = root_status;
            let _ = child.try_wait();
            let stopped_result = crate::process::stop(&mut child).await;
            cleanup_unconfirmed = stopped_result.is_err();
            Err(category)
        }
    };
    let cleanup = async {
        let (out, err) = tokio::join!(
            stdout.close(Duration::from_secs(5)),
            stderr.close(Duration::from_secs(5)),
        );
        out?;
        err?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match result {
        Ok(status) => {
            cleanup?;
            Ok(json!({"exit_code":status.code(),"success":status.success(),"stdout":out.text(),"stderr":err.text()}).to_string())
        }
        Err(category) => {
            if cleanup.is_err() {
                cleanup_unconfirmed = true;
            }
            Err(shell_failure_with_cleanup(
                category,
                cleanup_unconfirmed,
                &err,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_diagnostic::{ShellCapture, ShellFailureCategory, failure as shell_failure};
    use crate::test_support::{HttpFixture, Reply, drain_bounded};
    #[cfg(unix)]
    use crate::test_support::{StdioFixture, Step};
    use kuru_core::{McpConfig, PermissionAction, PermissionRule};

    fn with_test_checkpoints(host: ToolHost, private: &tempfile::TempDir) -> ToolHost {
        let root = host.root_guard.clone();
        host.with_checkpoint_store(Arc::new(
            CheckpointStore::new(&private.path().join("state"), root).unwrap(),
        ))
        .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn movable_parent_handoff_rejects_a_replacement_identity() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let root_guard =
            Directory::open(root.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let pinned = Directory::open(&parent, Privacy::Inherited, NameRetention::Pinned).unwrap();
        let expected_identity = pinned.identity();
        drop(pinned);

        std::fs::rename(&parent, root.path().join("displaced")).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let error = match reopen_movable_parent(&root_guard, &parent, expected_identity, "file") {
            Ok(_) => panic!("replacement parent was adopted"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "file parent changed during handle handoff"
        );
    }

    struct RecordInstructionDirectories(std::sync::Mutex<Vec<Vec<String>>>);

    #[async_trait::async_trait]
    impl InstructionGate for RecordInstructionDirectories {
        async fn review(
            &self,
            targets: &[ProjectRelativeTarget],
            _approval: Option<&InstructionReviewSender>,
        ) -> Result<InstructionGateOutcome> {
            self.0.lock().unwrap().push(
                targets
                    .iter()
                    .map(|target| target.as_str().to_owned())
                    .collect(),
            );
            Ok(InstructionGateOutcome::Required(
                "review before search exposure".into(),
            ))
        }
    }

    struct InstructionChangesAfterAdmission(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl InstructionGate for InstructionChangesAfterAdmission {
        async fn review(
            &self,
            _targets: &[ProjectRelativeTarget],
            _approval: Option<&InstructionReviewSender>,
        ) -> Result<InstructionGateOutcome> {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Ok(InstructionGateOutcome::Unchanged)
            } else {
                Ok(InstructionGateOutcome::Required(
                    "nested project instructions changed after admission".into(),
                ))
            }
        }
    }

    #[tokio::test]
    async fn cancelled_shell_capture_preserves_received_bytes_until_actual_eof() {
        use tokio::io::AsyncWriteExt;
        let (mut reader, mut writer) = tokio::io::duplex(1);
        let (sent, received) = tokio::sync::oneshot::channel();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let producer = tokio::spawn(async move {
            // The one-byte pipe cannot accept the suffix until the whole prefix
            // has been read. Then retain the writer to withhold real EOF.
            writer.write_all(b"accepted output!").await.unwrap();
            sent.send(()).unwrap();
            finishing.await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let mut capture = ShellCapture::new();
        timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = capture.read(&mut reader) => panic!("writer still open: {result:?}"),
                result = received => result.unwrap(),
            }
        })
        .await
        .unwrap();
        assert!(capture.bytes.starts_with(b"accepted output"));
        assert!(!capture.eof());
        finish.send(()).unwrap();
        timeout(Duration::from_secs(2), capture.read(&mut reader))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(capture.bytes, b"accepted output!");
        assert!(capture.eof());
        capture.finish().unwrap();
        assert_eq!(capture.text(), "accepted output!");
        producer.await.unwrap();
    }

    #[tokio::test]
    async fn shell_capture_retains_marked_head_and_tail_after_overflow() {
        let bytes = vec![b'x'; MAX_BYTES + 16384];
        let mut source = bytes.as_slice();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();
        let text = capture.text();
        assert!(text.len() <= MAX_BYTES);
        assert!(text.starts_with('x'));
        assert!(text.ends_with('x'));
        assert!(text.contains("[truncated]"));
        assert!(capture.eof());
    }

    #[tokio::test]
    async fn shell_capture_lossily_preserves_bounded_invalid_utf8_head_and_tail() {
        let mut bytes = b"HEAD\xff".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', MAX_BYTES));
        bytes.extend(b"\xfe:TAIL");
        let mut source = bytes.as_slice();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();
        let text = capture.text();
        assert!(text.len() <= MAX_BYTES);
        assert!(text.starts_with("HEAD?"));
        assert!(text.ends_with("?:TAIL"));
        assert!(text.contains("[truncated]"));
        assert!(capture.eof());
    }

    #[tokio::test]
    async fn shell_error_diagnostic_bounds_projected_stderr_without_reprojecting_marker() {
        let secret = "sk-proj-abcdefghijklmnop0123456789";
        let bytes = format!("openai_api_key={secret}\n{}:TAIL", "x".repeat(16 * 1024));
        let mut source = bytes.as_bytes();
        let mut capture = ShellCapture::new();
        capture.read(&mut source).await.unwrap();
        capture.finish().unwrap();

        let error = shell_failure(ShellFailureCategory::CaptureFailed, &capture);
        assert!(error.downcast_ref::<ProjectedShellDiagnostic>().is_some());
        assert_eq!(error.chain().count(), 1);
        let rendered = project_failure(ToolFailure::built_in(error))
            .unwrap_err()
            .to_string();
        assert!(rendered.contains("[REDACTED:recognized-secret]"));
        assert!(rendered.contains("[truncated]"));
        assert!(rendered.contains(":TAIL"));
        assert!(rendered.contains("shell capture failed; stderr: "));
        assert!(!rendered.contains(secret));
        let without_markers = rendered
            .replace("[REDACTED:recognized-secret]", "")
            .replace("[truncated]", "");
        assert!(
            !without_markers.contains(['[', ']']),
            "partial marker in {rendered:?}"
        );
    }

    #[tokio::test]
    async fn file_crud_is_contained_and_replaces_atomically() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "instructions").unwrap();
        std::fs::write(root.path().join(".env"), "protected").unwrap();
        std::fs::write(
            root.path().join(".kuru-edit-private-stage"),
            "private-stage-token",
        )
        .unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        let host = with_test_checkpoints(host, &private);
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|tool| tool.name == "file_write")
        );
        host.execute("file_write", json!({"path":"src/a.txt","content":"one"}))
            .await
            .unwrap();
        host.execute("file_write", json!({"path":"src/a.txt","content":"two"}))
            .await
            .unwrap();
        assert_eq!(
            host.execute("file_read", json!({"path":"./src/a.txt"}))
                .await
                .unwrap(),
            "two"
        );
        let entries: Value =
            serde_json::from_str(&host.execute("file_list", json!({})).await.unwrap()).unwrap();
        assert_eq!(entries["src"], "directory");
        assert!(entries.get(".env").is_none());
        assert!(entries.get(".kuru-edit-private-stage").is_none());
        for (name, args) in [
            (
                "grep",
                json!({"pattern":"private-stage-token","include_hidden":true}),
            ),
            (
                "glob",
                json!({"pattern":"**/.kuru-edit-*","include_hidden":true}),
            ),
        ] {
            let result: Value =
                serde_json::from_str(&host.execute(name, args).await.unwrap()).unwrap();
            assert_eq!(
                result["matches"],
                json!([]),
                "{name} exposed a private stage"
            );
        }
        assert_eq!(
            host.execute("file_read", json!({"path":"AGENTS.md"}))
                .await
                .unwrap(),
            "instructions"
        );
        for protected in [
            "../outside",
            "/etc/passwd",
            ".kuru/memory.sqlite",
            ".env",
            ".kuru-edit-private-stage",
            "src/../a",
            "",
        ] {
            assert!(
                host.execute("file_read", json!({"path":protected}))
                    .await
                    .is_err(),
                "{protected}"
            );
        }
        for protected in [
            "AGENTS.md",
            "CLAUDE.md",
            "mise.toml",
            "hk.pkl",
            ".mcp.json",
            ".codex/config.toml",
        ] {
            assert!(
                host.execute(
                    "file_write",
                    json!({"path":protected,"content":"overwritten"})
                )
                .await
                .is_err(),
                "{protected}"
            );
        }
        assert!(
            host.execute("file_write", json!({"path":"missing/child","content":"x"}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_write", json!({"path":"src","content":"x"}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_delete", json!({"path":"src"}))
                .await
                .is_err()
        );
        host.execute("file_delete", json!({"path":"src/a.txt"}))
            .await
            .unwrap();
        assert!(!root.path().join("src/a.txt").exists());
        assert!(
            host.execute("file_read", json!({"path":"src"}))
                .await
                .is_err()
        );
        assert!(host.execute("file_read", json!({"path":1})).await.is_err());
        assert!(host.execute("file_read", json!([])).await.is_err());
        assert!(host.execute("unknown", json!({})).await.is_err());
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn actor_file_edit_reuses_exact_receipt_and_selected_undo_restores_source() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("note.txt"), "alpha\none\nbeta\ntwo\n").unwrap();
        let host = with_test_checkpoints(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_write: true,
                    ..Default::default()
                },
            )
            .unwrap(),
            &private,
        );
        let context = ToolInvocationContext {
            session_id: "session-1".into(),
            turn_id: "turn-1".into(),
            actor_id: "actor-1".into(),
            invocation_id: "invocation-1".into(),
            call_id: "call-1".into(),
        };
        let arguments = json!({"path":"note.txt","hunks":[
            {"before":"alpha\n","old":"one","after":"\n","replacement":"un"},
            {"before":"beta\n","old":"two","after":"\n","replacement":"deux"}
        ]});
        let first = host
            .execute_for_actor_with_context("file_edit", arguments.clone(), None, None, &context)
            .await
            .result
            .unwrap();
        let id = first
            .strip_prefix("file_edit completed; checkpoint ")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("note.txt")).unwrap(),
            "alpha\nun\nbeta\ndeux\n"
        );
        let retry = host
            .execute_for_actor_with_context("file_edit", arguments.clone(), None, None, &context)
            .await
            .result
            .unwrap();
        assert_eq!(retry, first);
        assert!(
            host.execute_for_actor_with_context(
                "file_edit",
                json!({"path":"note.txt","hunks":[
                    {"before":"alpha\n","old":"one","after":"\n","replacement":"changed"}
                ]}),
                None,
                None,
                &context
            )
            .await
            .result
            .unwrap_err()
            .to_string()
            .contains("conflicts")
        );
        let undone = host.undo_file_checkpoint(id, None).await.unwrap();
        assert_eq!(undone.effect, FileEffect::Undo);
        assert_eq!(
            std::fs::read_to_string(root.path().join("note.txt")).unwrap(),
            "alpha\none\nbeta\ntwo\n"
        );
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn offline_actor_edit_proposals_apply_only_the_unique_authorized_hunk() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("valid.txt"), "alpha old omega\n").unwrap();
        std::fs::write(root.path().join("ambiguous.txt"), "old old\n").unwrap();
        std::fs::write(root.path().join("stale.txt"), "new source\n").unwrap();
        std::fs::write(root.path().join(".env"), "protected secret\n").unwrap();
        let host = with_test_checkpoints(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_write: true,
                    ..Default::default()
                },
            )
            .unwrap(),
            &private,
        );
        // Fixed, provider-free tool proposals use separate admitted call IDs.
        // A refusal cannot be hidden by exact-retry receipt matching.
        let proposals = [
            (
                "valid.txt",
                json!([{"before":"alpha ","old":"old","after":" omega\n","replacement":"new"}]),
                None,
            ),
            (
                "ambiguous.txt",
                json!([{"before":"","old":"old","after":"","replacement":"new"}]),
                Some("ambiguous"),
            ),
            (
                "stale.txt",
                json!([{"before":"","old":"old","after":"","replacement":"new"}]),
                Some("stale"),
            ),
            (
                ".env",
                json!([{"before":"","old":"protected","after":"","replacement":"new"}]),
                Some("protected"),
            ),
        ];
        for (index, (path, hunks, refused_for)) in proposals.into_iter().enumerate() {
            let context = ToolInvocationContext {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                actor_id: "actor-1".into(),
                invocation_id: "invocation-1".into(),
                call_id: format!("call-{index}"),
            };
            let result = host
                .execute_for_actor_with_context(
                    "file_edit",
                    json!({"path":path,"hunks":hunks}),
                    None,
                    None,
                    &context,
                )
                .await
                .result;
            if let Some(reason) = refused_for {
                let error = result.unwrap_err().to_string();
                assert!(error.contains(reason), "{path}: {error}");
            } else {
                assert!(
                    result
                        .unwrap()
                        .starts_with("file_edit completed; checkpoint "),
                    "valid edit did not settle"
                );
            }
        }
        assert_eq!(
            std::fs::read_to_string(root.path().join("valid.txt")).unwrap(),
            "alpha new omega\n"
        );
        for (path, expected) in [
            ("ambiguous.txt", "old old\n"),
            ("stale.txt", "new source\n"),
            (".env", "protected secret\n"),
        ] {
            assert_eq!(
                std::fs::read_to_string(root.path().join(path)).unwrap(),
                expected
            );
        }
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn native_search_and_paged_read_are_checked_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/utf8.txt"),
            "first\nβeta needle\nlast\n",
        )
        .unwrap();
        std::fs::write(root.path().join(".hidden.txt"), "needle\n").unwrap();
        std::fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.path().join("ignored.txt"), "needle\n").unwrap();
        std::fs::write(root.path().join("binary.bin"), b"needle\xff").unwrap();
        std::fs::write(
            root.path().join("oversized-line.txt"),
            format!("needle{}\n", "x".repeat(MAX_SEARCH_LINE_BYTES)),
        )
        .unwrap();
        std::fs::write(
            root.path().join("oversized-file.txt"),
            "x".repeat(MAX_SEARCH_FILE_BYTES as usize + 1),
        )
        .unwrap();
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();

        let specs = host.specs().await.unwrap();
        assert!(specs.iter().any(|spec| spec.name == "grep"));
        assert!(specs.iter().any(|spec| spec.name == "glob"));
        let read = specs.iter().find(|spec| spec.name == "file_read").unwrap();
        assert_eq!(read.parameters["properties"]["offset"]["minimum"], 1);
        assert_eq!(read.parameters["properties"]["limit"]["maximum"], 10_000);

        let grep: Value = serde_json::from_str(
            &host
                .execute("grep", json!({"pattern":"needle"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(grep["matches"][0]["path"], "src/utf8.txt");
        assert_eq!(grep["matches"][0]["line"], 2);
        assert_eq!(grep["matches"].as_array().unwrap().len(), 1);
        assert_eq!(grep["omitted"]["oversized_line"], 1);
        assert_eq!(grep["omitted"]["large_or_non_file"], 1);
        assert_eq!(grep["omitted"]["unreadable"], 1);

        let ignored_only: Value = serde_json::from_str(
            &host
                .execute("glob", json!({"pattern":"**/*.txt","include_ignored":true}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            ignored_only["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == "ignored.txt")
        );
        assert!(
            !ignored_only["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == ".hidden.txt")
        );
        let hidden_only: Value = serde_json::from_str(
            &host
                .execute("glob", json!({"pattern":"**/*.txt","include_hidden":true}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            hidden_only["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == ".hidden.txt")
        );
        assert!(
            !hidden_only["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == "ignored.txt")
        );

        let page: Value = serde_json::from_str(
            &host
                .execute(
                    "file_read",
                    json!({"path":"src/utf8.txt","offset":2,"limit":1}),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(page["text"], "βeta needle\n");
        assert_eq!(page["next_offset"], 3);
        assert_eq!(page["omitted_lines"], 1);
        assert!(
            host.execute("file_read", json!({"path":"binary.bin","limit":1}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_read", json!({"path":"src/utf8.txt","offset":0}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_read", json!({"path":"src/utf8.txt","offset":"two"}))
                .await
                .is_err()
        );
        assert!(
            host.execute("file_read", json!({"path":"src/utf8.txt","limit":null}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn search_authorizes_each_candidate_without_widening_a_root_request() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("allowed.txt"), "needle\n").unwrap();
        std::fs::write(root.path().join("denied.txt"), "needle\n").unwrap();
        let selector = PermissionSelector::native(NativeTool::Glob);
        let host = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: selector.clone(),
                    path: Some("denied.txt".into()),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let result: Value = serde_json::from_str(
            &host
                .execute("glob", json!({"pattern":"*.txt"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["matches"], json!(["allowed.txt"]));
        assert_eq!(result["omitted"]["denied"], 1);

        let asked = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Ask,
                        selector,
                        path: Some("allowed.txt".into()),
                    }],
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let pending = {
            let asked = Arc::clone(&asked);
            tokio::spawn(async move {
                asked
                    .execute_with_approval(
                        "glob",
                        json!({"pattern":"allowed.txt"}),
                        Some(&crate::ApprovalSender::new(sender)),
                    )
                    .await
            })
        };
        let request = receiver.recv().await.unwrap();
        assert_eq!(request.scope.target().unwrap().as_str(), "allowed.txt");
        request.reply.send(crate::ApprovalAnswer::Once).unwrap();
        let approved: Value = serde_json::from_str(&pending.await.unwrap().unwrap()).unwrap();
        assert_eq!(approved["matches"], json!(["allowed.txt"]));

        let denied = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::Glob),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let error = denied
            .execute("glob", json!({"pattern":"*.txt"}))
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
    }

    #[tokio::test]
    async fn actor_search_reviews_only_allowed_candidate_directories_before_exposure() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::create_dir(root.path().join("sibling")).unwrap();
        std::fs::write(root.path().join("src/allowed.txt"), "needle\n").unwrap();
        std::fs::write(root.path().join("sibling/denied.txt"), "needle\n").unwrap();
        let gate = Arc::new(RecordInstructionDirectories(std::sync::Mutex::new(
            Vec::new(),
        )));
        let host = ToolHost::new(
            root.path(),
            &Config {
                permissions: [NativeTool::Glob, NativeTool::Grep]
                    .into_iter()
                    .map(|tool| PermissionRule {
                        action: PermissionAction::Deny,
                        selector: PermissionSelector::native(tool),
                        path: Some("sibling/denied.txt".into()),
                    })
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap()
        .with_instruction_gate(gate.clone());
        for (name, args) in [
            ("glob", json!({"pattern":"**/*.txt"})),
            ("grep", json!({"pattern":"needle"})),
        ] {
            let result = host.execute_for_actor(name, args, None, None).await;
            assert!(
                result
                    .result
                    .unwrap_err()
                    .to_string()
                    .contains("review before search exposure")
            );
            assert!(result.instructions.is_none());
        }
        assert_eq!(
            *gate.0.lock().unwrap(),
            vec![vec!["src".to_owned()], vec!["src".to_owned()]]
        );
    }

    #[tokio::test]
    async fn parallel_read_admission_is_narrow_and_preserves_serial_authority_boundaries() {
        let context = |call_id: &str| ToolInvocationContext {
            session_id: "session-parallel".into(),
            turn_id: "turn-parallel".into(),
            actor_id: "actor-parallel".into(),
            invocation_id: "invocation-parallel".into(),
            call_id: call_id.into(),
        };
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "needle\n").unwrap();
        let host = Arc::new(ToolHost::new(root.path(), &Config::default()).unwrap());

        let ParallelReadAdmission::Ready(read) = host
            .prepare_parallel_read("file_read", json!({"path":"a.txt"}), context("read-a"))
            .await
        else {
            panic!("ordinary file read should enter the prepared class");
        };
        assert_eq!(
            read.execute(host.clone(), ParallelReadCancellation::default())
                .await
                .result
                .unwrap(),
            "needle\n"
        );
        let ParallelReadAdmission::Ready(cancelled) = host
            .prepare_parallel_read("file_list", json!({"path":"."}), context("cancel-list"))
            .await
        else {
            panic!("ordinary file list should enter the prepared class");
        };
        let cancellation = ParallelReadCancellation::default();
        cancellation.cancel();
        assert!(
            cancelled
                .execute(host.clone(), cancellation)
                .await
                .result
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert_eq!(host.parallel_read_handle_usage_for_test().0, 0);

        std::fs::create_dir(root.path().join("many")).unwrap();
        for index in 0..40 {
            std::fs::write(
                root.path().join("many").join(format!("match-{index}.txt")),
                "bounded-search-match\n",
            )
            .unwrap();
        }
        assert!(matches!(
            host.prepare_parallel_read(
                "grep",
                json!({"path":"many","pattern":"bounded-search-match"}),
                context("serial-handle-fallback"),
            )
            .await,
            ParallelReadAdmission::Serial
        ));
        assert_eq!(host.parallel_read_handle_usage_for_test().0, 0);
        let ordinary: Value = serde_json::from_str(
            &host
                .execute(
                    "grep",
                    json!({"path":"many","pattern":"bounded-search-match"}),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ordinary["matches"].as_array().unwrap().len(), 40);
        std::fs::remove_dir_all(root.path().join("many")).unwrap();

        let mut retained = Vec::new();
        let mut saw_serial = false;
        for index in 0..64 {
            match host
                .prepare_parallel_read(
                    "file_read",
                    json!({"path":"a.txt"}),
                    context(&format!("bounded-handle-{index}")),
                )
                .await
            {
                ParallelReadAdmission::Ready(read) => retained.push(read),
                ParallelReadAdmission::Serial => {
                    saw_serial = true;
                    break;
                }
            }
        }
        let (used, maximum) = host.parallel_read_handle_usage_for_test();
        assert!(saw_serial && used > 0 && used <= maximum);
        drop(retained);
        assert_eq!(host.parallel_read_handle_usage_for_test().0, 0);
        assert!(matches!(
            host.prepare_parallel_read(
                "file_write",
                json!({"path":"b.txt","content":"x"}),
                context("serial-write"),
            )
            .await,
            ParallelReadAdmission::Serial
        ));
        // web_fetch asks by default. A fresh foreground/Once answer cannot be
        // represented by a prepared call and stays on immediate serial dispatch.
        assert!(matches!(
            host.prepare_parallel_read(
                "web_fetch",
                json!({"url":"https://example.com"}),
                context("serial-web"),
            )
            .await,
            ParallelReadAdmission::Serial
        ));
        let web = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Allow,
                        selector: PermissionSelector::native(NativeTool::WebFetch),
                        path: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap()
            .with_parallel_read_barrier_for_test(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let ParallelReadAdmission::Ready(first_web) = web
            .prepare_parallel_read(
                "web_fetch",
                json!({"url":"http://127.0.0.1/first"}),
                context("web-first"),
            )
            .await
        else {
            panic!("explicitly authorized web fetch should enter the prepared class");
        };
        let ParallelReadAdmission::Ready(second_web) = web
            .prepare_parallel_read(
                "web_fetch",
                json!({"url":"http://127.0.0.1/second"}),
                context("web-second"),
            )
            .await
        else {
            panic!("second authorized web fetch should enter the prepared class");
        };
        let (first_web, second_web) = tokio::join!(
            first_web.execute(web.clone(), ParallelReadCancellation::default()),
            second_web.execute(web, ParallelReadCancellation::default())
        );
        for outcome in [first_web, second_web] {
            let error = outcome.result.unwrap_err();
            assert!(!crate::is_permission_denied(&error));
            assert!(error.to_string().contains("not a public address"));
        }

        let asked = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Ask,
                        selector: PermissionSelector::native(NativeTool::FileRead),
                        path: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        assert!(matches!(
            asked
                .prepare_parallel_read(
                    "file_read",
                    json!({"path":"a.txt"}),
                    context("asked-before-once"),
                )
                .await,
            ParallelReadAdmission::Serial
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let once = {
            let asked = asked.clone();
            tokio::spawn(async move {
                asked
                    .execute_with_approval(
                        "file_read",
                        json!({"path":"a.txt"}),
                        Some(&crate::ApprovalSender::new(sender)),
                    )
                    .await
            })
        };
        receiver
            .recv()
            .await
            .unwrap()
            .reply
            .send(crate::ApprovalAnswer::Once)
            .unwrap();
        assert_eq!(once.await.unwrap().unwrap(), "needle\n");
        assert!(matches!(
            asked
                .prepare_parallel_read(
                    "file_read",
                    json!({"path":"a.txt"}),
                    context("asked-after-once"),
                )
                .await,
            ParallelReadAdmission::Serial
        ));

        let denied = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::FileRead),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            denied
                .prepare_parallel_read(
                    "file_read",
                    json!({"path":"a.txt"}),
                    context("denied-read"),
                )
                .await,
            ParallelReadAdmission::Serial
        ));
        assert!(crate::is_permission_denied(
            &denied
                .execute("file_read", json!({"path":"a.txt"}))
                .await
                .unwrap_err()
        ));

        let gate = Arc::new(RecordInstructionDirectories(std::sync::Mutex::new(
            Vec::new(),
        )));
        let reviewed = ToolHost::new(root.path(), &Config::default())
            .unwrap()
            .with_instruction_gate(gate);
        assert!(matches!(
            reviewed
                .prepare_parallel_read(
                    "grep",
                    json!({"pattern":"needle"}),
                    context("reviewed-grep"),
                )
                .await,
            ParallelReadAdmission::Serial
        ));

        let changed = Arc::new(
            ToolHost::new(root.path(), &Config::default())
                .unwrap()
                .with_instruction_gate(Arc::new(InstructionChangesAfterAdmission(
                    std::sync::atomic::AtomicUsize::new(0),
                ))),
        );
        let ParallelReadAdmission::Ready(read) = changed
            .prepare_parallel_read(
                "file_read",
                json!({"path":"a.txt"}),
                context("changed-instructions"),
            )
            .await
        else {
            panic!("unchanged instruction graph should admit the read");
        };
        let error = read
            .execute(changed, ParallelReadCancellation::default())
            .await
            .result
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(error.to_string().contains("replan this call"));

        std::fs::write(root.path().join("replace.txt"), "admitted bytes").unwrap();
        let target_changed = Arc::new(ToolHost::new(root.path(), &Config::default()).unwrap());
        let ParallelReadAdmission::Ready(read) = target_changed
            .prepare_parallel_read(
                "file_read",
                json!({"path":"replace.txt"}),
                context("changed-target"),
            )
            .await
        else {
            panic!("the original checked target should be admitted");
        };
        std::fs::remove_file(root.path().join("replace.txt")).unwrap();
        std::fs::write(root.path().join("replace.txt"), "replacement bytes").unwrap();
        let error = read
            .execute(target_changed.clone(), ParallelReadCancellation::default())
            .await
            .result
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(error.to_string().contains("replan this call"));

        std::fs::create_dir(root.path().join("listed")).unwrap();
        std::fs::write(root.path().join("listed/item.txt"), "admitted").unwrap();
        let ParallelReadAdmission::Ready(list) = target_changed
            .prepare_parallel_read(
                "file_list",
                json!({"path":"listed"}),
                context("changed-directory"),
            )
            .await
        else {
            panic!("the original checked directory should be admitted");
        };
        std::fs::rename(root.path().join("listed"), root.path().join("old-listed")).unwrap();
        std::fs::create_dir(root.path().join("listed")).unwrap();
        let error = list
            .execute(target_changed.clone(), ParallelReadCancellation::default())
            .await
            .result
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(error.to_string().contains("replan this call"));

        std::fs::write(root.path().join("search.txt"), "unique-admitted-pattern").unwrap();
        let ParallelReadAdmission::Ready(grep) = target_changed
            .prepare_parallel_read(
                "grep",
                json!({"pattern":"unique-admitted-pattern"}),
                context("changed-search-candidate"),
            )
            .await
        else {
            panic!("the original checked search candidates should be admitted");
        };
        std::fs::remove_file(root.path().join("search.txt")).unwrap();
        std::fs::write(root.path().join("search.txt"), "replacement search bytes").unwrap();
        let error = grep
            .execute(target_changed, ParallelReadCancellation::default())
            .await
            .result
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(error.to_string().contains("replan this call"));
    }

    #[tokio::test]
    async fn central_permission_gate_denies_effects_and_explicit_allow_overrides_legacy_false() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let denied_path = root.path().join("denied.txt");
        let denied = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::FileWrite),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !denied
                .specs()
                .await
                .unwrap()
                .iter()
                .any(|tool| tool.name == "file_write")
        );
        let error = denied
            .execute(
                "file_write",
                json!({"path":"denied.txt","content":"must not exist"}),
            )
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(!denied_path.exists());

        let allowed = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Allow,
                    selector: PermissionSelector::native(NativeTool::FileWrite),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let allowed = with_test_checkpoints(allowed, &private);
        allowed
            .execute(
                "file_write",
                json!({"path":"allowed.txt","content":"written"}),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("allowed.txt")).unwrap(),
            "written"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn retained_windows_root_spellings_reach_file_permission_decisions() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let ordinary = root.path().to_path_buf();
        let verbatim = root.path().canonicalize().unwrap();

        for (index, spelling) in [ordinary, verbatim].into_iter().enumerate() {
            let retained = Arc::new(
                Directory::open(&spelling, Privacy::Inherited, NameRetention::Pinned).unwrap(),
            );
            let allowed_name = format!("allowed-{index}.txt");
            let allowed = ToolHost::with_retained_root(
                retained.clone(),
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Allow,
                        selector: PermissionSelector::native(NativeTool::FileWrite),
                        path: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            let allowed = with_test_checkpoints(allowed, &private);
            allowed
                .execute(
                    "file_write",
                    json!({"path": &allowed_name, "content": "written"}),
                )
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(root.path().join(&allowed_name)).unwrap(),
                "written"
            );

            let asked_name = format!("asked-{index}.txt");
            let asked = ToolHost::with_retained_root(
                retained,
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Ask,
                        selector: PermissionSelector::native(NativeTool::FileWrite),
                        path: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
            let refused = asked
                .execute(
                    "file_write",
                    json!({"path": &asked_name, "content": "must not be written"}),
                )
                .await
                .unwrap_err();
            assert!(
                crate::is_permission_denied(&refused),
                "unexpected projected tool error: {refused:#}"
            );
            assert!(!root.path().join(asked_name).exists());
            asked.shutdown().await.unwrap();
            allowed.shutdown().await.unwrap();
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[tokio::test]
    async fn physical_case_alias_cannot_bypass_file_permission_path_deny() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("Secret.txt");
        std::fs::write(&actual, "original").unwrap();

        // This fixture only applies when the host filesystem resolves a case
        // alias. APFS and the usual Windows volumes do; a case-sensitive
        // workspace has no alias to exercise.
        let alias = root.path().join("secret.txt");
        if !alias.exists() {
            eprintln!("host filesystem has no case alias; fixture is inapplicable");
            return;
        }
        eprintln!("host filesystem resolved Secret.txt through secret.txt");

        let host = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::FileWrite),
                    path: Some("Secret.txt".into()),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let error = host
            .execute(
                "file_write",
                json!({"path":"secret.txt","content":"must not replace"}),
            )
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert_eq!(std::fs::read_to_string(actual).unwrap(), "original");
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[tokio::test]
    async fn changed_physical_target_while_approval_is_pending_has_no_effect() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("Secret.txt");
        std::fs::write(&actual, "original").unwrap();
        let alias = root.path().join("secret.txt");
        if !alias.exists() {
            eprintln!("host filesystem has no case alias; fixture is inapplicable");
            return;
        }

        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    permissions: vec![PermissionRule {
                        action: PermissionAction::Ask,
                        selector: PermissionSelector::native(NativeTool::FileWrite),
                        path: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<crate::ApprovalRequest>(1);
        let pending = {
            let host = Arc::clone(&host);
            tokio::spawn(async move {
                host.execute_with_approval(
                    "file_write",
                    json!({"path":"secret.txt","content":"must not replace"}),
                    Some(&crate::ApprovalSender::new(sender)),
                )
                .await
            })
        };
        let request = receiver.recv().await.unwrap();
        std::fs::rename(&actual, root.path().join("previous.txt")).unwrap();
        std::fs::write(&alias, "replacement").unwrap();
        request.reply.send(crate::ApprovalAnswer::Once).unwrap();

        let error = pending.await.unwrap().unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert_eq!(std::fs::read_to_string(alias).unwrap(), "replacement");
        assert_eq!(
            std::fs::read_to_string(root.path().join("previous.txt")).unwrap(),
            "original"
        );
    }

    #[tokio::test]
    async fn foreground_once_approval_dispatches_the_bound_native_write() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let host = with_test_checkpoints(
            ToolHost::new(root.path(), &Config::default()).unwrap(),
            &private,
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<crate::ApprovalRequest>(1);
        let reply = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert!(request.display.rememberable);
            assert_eq!(request.scope.target().unwrap().as_str(), "approved.txt");
            request.reply.send(crate::ApprovalAnswer::Once).unwrap();
        });
        host.execute_with_approval(
            "file_write",
            json!({"path":"approved.txt","content":"approved"}),
            Some(&crate::ApprovalSender::new(sender)),
        )
        .await
        .unwrap();
        reply.await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("approved.txt")).unwrap(),
            "approved"
        );
        assert!(
            host.execute("file_write", json!({"path":"retry.txt","content":"retry"}))
                .await
                .is_err(),
            "once approval did not survive a new invocation"
        );
        assert!(!root.path().join("retry.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn central_permission_gate_denies_shell_before_process_spawn() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("shell-denied");
        let host = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::Shell),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let error = host
            .execute(
                "shell",
                json!({"command":format!("touch {}", marker.display())}),
            )
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_once_approval_dispatches_shell() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("shell-approved");
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<crate::ApprovalRequest>(1);
        let reply = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert_eq!(request.display.label, "native shell");
            request.reply.send(crate::ApprovalAnswer::Once).unwrap();
        });
        host.execute_with_approval(
            "shell",
            json!({"command":format!("touch {}", marker.display())}),
            Some(&crate::ApprovalSender::new(sender)),
        )
        .await
        .unwrap();
        reply.await.unwrap();
        assert!(marker.exists());
    }

    #[tokio::test]
    async fn web_fetch_uses_the_normal_allow_ask_and_deny_receipt_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let arguments = json!({"url":"http://127.0.0.1/fixture"});

        let allowed = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Allow,
                    selector: PermissionSelector::native(NativeTool::WebFetch),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let error = allowed
            .execute("web_fetch", arguments.clone())
            .await
            .unwrap_err();
        assert!(
            !crate::is_permission_denied(&error),
            "allow did not dispatch to the destination policy: {error:#}"
        );
        assert!(format!("{error:#}").contains("not a public address"));

        let denied = ToolHost::new(
            root.path(),
            &Config {
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::native(NativeTool::WebFetch),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(crate::is_permission_denied(
            &denied
                .execute("web_fetch", arguments.clone())
                .await
                .unwrap_err()
        ));

        let asked = ToolHost::new(root.path(), &Config::default()).unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<crate::ApprovalRequest>(1);
        let reply = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert_eq!(request.display.label, "native web fetch");
            request.reply.send(crate::ApprovalAnswer::Deny).unwrap();
        });
        assert!(crate::is_permission_denied(
            &asked
                .execute_with_approval(
                    "web_fetch",
                    arguments,
                    Some(&crate::ApprovalSender::new(sender))
                )
                .await
                .unwrap_err()
        ));
        reply.await.unwrap();
    }

    #[tokio::test]
    async fn central_permission_gate_denies_http_mcp_before_tools_call() {
        let mut initialized =
            Reply::rpc(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}));
        initialized.session = true;
        let peer = HttpFixture::new(vec![
            initialized,
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[{"name":"remote","inputSchema":{"type":"object"}}]})),
            Reply::json(json!({})),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "http".into(),
                    McpConfig {
                        command: None,
                        args: vec![],
                        url: Some(peer.url.clone()),
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::mcp("http", "remote").unwrap(),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .all(|spec| !spec.name.starts_with("mcp_"))
        );
        let name = format!(
            "mcp_{}",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, b"http\0remote").simple()
        );
        let error = host
            .execute(&name, json!({"write":"blocked"}))
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        host.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 4);
        assert!(
            requests
                .iter()
                .all(|request| request.body["method"] != "tools/call")
        );
    }

    #[tokio::test]
    async fn a2a_admission_uses_the_same_typed_denial_before_http() {
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                external_agents: [("remote".into(), "http://127.0.0.1:1".into())].into(),
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::a2a("remote").unwrap(),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let error = host
            .authorize_a2a(
                "remote",
                "http://127.0.0.1:1",
                &json!({"agent":"remote","message":"blocked"}),
                None,
            )
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
    }

    #[tokio::test]
    async fn tool_projection_redacts_results_without_altering_files_or_arguments() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        const MARKER: &str = "[REDACTED:recognized-secret]";
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let filename = "read.txt";
        let source = format!("ordinary {SECRET} retained on disk");
        std::fs::write(root.path().join(filename), &source).unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        let host = with_test_checkpoints(host, &private);

        let read = host
            .execute("file_read", json!({"path":&filename}))
            .await
            .unwrap();
        assert!(read.contains(MARKER));
        assert!(!read.contains(SECRET));
        assert_eq!(
            std::fs::read_to_string(root.path().join(filename)).unwrap(),
            source
        );

        let written = "write.txt";
        host.execute("file_write", json!({"path":&written,"content":&source}))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join(written)).unwrap(),
            source
        );

        let listing = host.execute("file_list", json!({})).await.unwrap();
        let listing: Value = serde_json::from_str(&listing).unwrap();
        assert_eq!(listing[filename], "file");
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn file_read_accepts_exactly_the_existing_byte_limit() {
        let root = tempfile::tempdir().unwrap();
        let contents = "x".repeat(MAX_BYTES);
        std::fs::write(root.path().join("at-limit"), &contents).unwrap();
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        assert_eq!(
            host.execute("file_read", json!({"path":"at-limit"}))
                .await
                .unwrap(),
            contents
        );
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_utf8_file_failure_withholds_original_bytes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("bad"), b"sk-abcdefghijklmnop\xff").unwrap();
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        let error = host
            .execute("file_read", json!({"path":"bad"}))
            .await
            .unwrap_err();
        for rendered in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
        ] {
            assert!(rendered.contains("file is not UTF-8"));
            assert!(!rendered.contains("sk-abcdefghijklmnop"));
        }
        assert_eq!(error.chain().count(), 1);
        assert!(error.downcast_ref::<std::string::FromUtf8Error>().is_none());
        host.shutdown().await.unwrap();
    }

    #[test]
    fn projected_tool_failures_do_not_retain_raw_error_sources() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let error =
            anyhow::anyhow!("outer failure {SECRET}").context(format!("inner failure {SECRET}"));
        let error = project_failure(ToolFailure::built_in(error)).unwrap_err();
        for rendered in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ] {
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        assert_eq!(error.chain().count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn toolhost_stdio_mcp_projects_application_success_and_protocol_results() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote","inputSchema":{"type":"object"}}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"isError":true,"content":[{"type":"text","text":format!("denied {SECRET}")}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":format!("usable {SECRET}")}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":5,"error":{"code":-32000,"message":format!("failed {SECRET}")}}),
            ),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "fixture".into(),
                    McpConfig {
                        command: Some(peer.command().into()),
                        args: vec![],
                        url: None,
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP fixture/remote:"))
            .expect("missing discovered fixture MCP tool")
            .name;
        let application = host.execute(&name, json!({})).await.unwrap_err();
        for rendered in [
            application.to_string(),
            format!("{application:#}"),
            format!("{application:?}"),
        ] {
            assert!(rendered.contains("denied"));
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        let success = host.execute(&name, json!({})).await.unwrap();
        assert!(success.contains("[REDACTED:recognized-secret]"));
        assert!(!success.contains(SECRET));
        let failure = host.execute(&name, json!({})).await.unwrap_err();
        assert_eq!(
            failure.to_string(),
            "MCP tool call failed: configured MCP server fixture is unavailable"
        );
        assert!(!format!("{failure:#} {failure:?}").contains(SECRET));
        assert_eq!(failure.chain().count(), 1);
        host.shutdown().await.unwrap();
        assert_eq!(
            peer.conversations(),
            vec![vec![
                json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "method":"initialize",
                    "params":{
                        "protocolVersion":"2025-11-25",
                        "capabilities":{},
                        "clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")},
                    },
                }),
                json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
                json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
                json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"remote","arguments":{}}}),
            ]],
            "the protocol-error cleanup may terminate the peer before its planned EOF, but it must not skip or replay a request"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn central_permission_gate_denies_stdio_mcp_before_tools_call() {
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote","inputSchema":{"type":"object"}}]}}),
            ),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "fixture".into(),
                    McpConfig {
                        command: Some(peer.command().into()),
                        args: vec![],
                        url: None,
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                permissions: vec![PermissionRule {
                    action: PermissionAction::Deny,
                    selector: PermissionSelector::mcp("fixture", "remote").unwrap(),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .all(|spec| !spec.name.starts_with("mcp_"))
        );
        let name = format!(
            "mcp_{}",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, b"fixture\0remote").simple()
        );
        let error = host
            .execute(&name, json!({"write":"blocked"}))
            .await
            .unwrap_err();
        assert!(crate::is_permission_denied(&error));
        host.shutdown().await.unwrap();
        assert_eq!(
            peer.conversations(),
            vec![vec![
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"kuru","version":env!("CARGO_PKG_VERSION")}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
            ]]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_once_approval_dispatches_stdio_mcp_tool_call() {
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"remote","inputSchema":{"type":"object"}}]}}),
            ),
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"approved"}]}}),
            ),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "fixture".into(),
                    McpConfig {
                        command: Some(peer.command().into()),
                        args: vec![],
                        url: None,
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                permissions: vec![PermissionRule {
                    action: PermissionAction::Ask,
                    selector: PermissionSelector::mcp("fixture", "remote").unwrap(),
                    path: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP fixture/remote:"))
            .unwrap()
            .name;
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<crate::ApprovalRequest>(1);
        let reply = tokio::spawn(async move {
            let request = receiver.recv().await.unwrap();
            assert!(request.scope.target().is_none());
            request.reply.send(crate::ApprovalAnswer::Once).unwrap();
        });
        assert!(
            host.execute_with_approval(
                &name,
                json!({"write":"approved"}),
                Some(&crate::ApprovalSender::new(sender)),
            )
            .await
            .unwrap()
            .contains("approved")
        );
        reply.await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(
            peer.conversations()[0][3],
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remote","arguments":{"write":"approved"}}})
        );
    }

    #[tokio::test]
    async fn toolhost_http_mcp_projects_application_success_and_protocol_results() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let mut initialized =
            Reply::rpc(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}));
        initialized.session = true;
        let peer = HttpFixture::new(vec![
            initialized,
            Reply::json(json!({})),
            Reply::rpc(json!({"tools":[{"name":"remote","description":"fixture","inputSchema":{"type":"object"}}]})),
            Reply::rpc(json!({"isError":true,"content":[{"type":"text","text":format!("denied {SECRET}")}]})),
            Reply::rpc(json!({"content":[{"type":"text","text":format!("usable {SECRET}")}],"structuredContent":{"nested":{"value":7}}})),
            Reply::json(json!({"id":"$ID","error":{"code":-32000,"message":format!("failed {SECRET}")}})),
            Reply::json(json!({})),
        ]).await;
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "http".into(),
                    McpConfig {
                        command: None,
                        args: vec![],
                        url: Some(peer.url.clone()),
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP http/remote:"))
            .expect("missing discovered HTTP MCP tool")
            .name;
        let application = host
            .execute(&name, json!({"input": SECRET}))
            .await
            .unwrap_err();
        for rendered in [
            application.to_string(),
            format!("{application:#}"),
            format!("{application:?}"),
        ] {
            assert!(rendered.contains("denied"));
            assert!(rendered.contains("[REDACTED:recognized-secret]"));
            assert!(!rendered.contains(SECRET));
        }
        let success: Value =
            serde_json::from_str(&host.execute(&name, json!({})).await.unwrap()).unwrap();
        assert_eq!(success["structuredContent"]["nested"]["value"], 7);
        assert_eq!(
            success["content"][0]["text"],
            "usable [REDACTED:recognized-secret]"
        );
        let failure = host.execute(&name, json!({})).await.unwrap_err();
        for rendered in [
            failure.to_string(),
            format!("{failure:#}"),
            format!("{failure:?}"),
        ] {
            assert!(rendered.contains("configured MCP server http is unavailable"));
            assert!(!rendered.contains(SECRET));
        }
        assert_eq!(failure.chain().count(), 1);
        host.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[3].body["method"], "tools/call");
        assert_eq!(
            requests[3].body["params"]["arguments"],
            json!({"input": SECRET})
        );
    }

    #[tokio::test]
    async fn file_list_keeps_its_independent_entry_cap_above_result_limit() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..10_000 {
            let name = format!("{index:05}-{}", "x".repeat(238));
            std::fs::write(root.path().join(name), []).unwrap();
        }
        let host = ToolHost::new(root.path(), &Config::default()).unwrap();
        let listed = host.execute("file_list", json!({})).await.unwrap();
        assert!(listed.len() > MAX_BYTES);
        let listed: Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed.as_object().unwrap().len(), 10_000);
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn toolhost_http_mcp_accepts_an_exact_wire_body_limit() {
        let mut initialized =
            Reply::rpc(json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}));
        initialized.session = true;
        let empty = Reply::rpc(json!({"content":[{"type":"text","text":""}]}));
        let payload = "x".repeat(MAX_BYTES - empty.body.replace("\"$ID\"", "3").len());
        let reply = Reply::rpc(json!({"content":[{"type":"text","text":payload}]}));
        assert_eq!(reply.body.replace("\"$ID\"", "3").len(), MAX_BYTES);
        let peer = HttpFixture::new(vec![initialized, Reply::json(json!({})), Reply::rpc(json!({"tools":[{"name":"remote","description":"fixture","inputSchema":{"type":"object"}}]})), reply, Reply::json(json!({}))]).await;
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                mcp: [(
                    "http".into(),
                    McpConfig {
                        command: None,
                        args: vec![],
                        url: Some(peer.url.clone()),
                        env: BTreeMap::new(),
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        let name = host
            .specs()
            .await
            .unwrap()
            .into_iter()
            .find(|spec| spec.description.starts_with("MCP http/remote:"))
            .unwrap()
            .name;
        let value: Value =
            serde_json::from_str(&host.execute(&name, json!({})).await.unwrap()).unwrap();
        host.shutdown().await.unwrap();
        let requests = peer.requests.lock().await;
        assert_eq!(requests[3].body["id"], 3);
        assert_eq!(value, json!({"content":[{"type":"text","text":payload}]}));
    }

    #[tokio::test]
    async fn permissions_and_file_reads_preserve_bounded_visible_output() {
        let root = tempfile::tempdir().unwrap();
        let read_only = ToolHost::new(root.path(), &Config::default()).unwrap();
        let names: Vec<_> = read_only
            .specs()
            .await
            .unwrap()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "file_read",
                "file_list",
                "grep",
                "glob",
                "file_write",
                "file_edit",
                "file_delete",
                "web_fetch",
                "shell"
            ]
        );
        for name in ["file_write", "file_delete", "shell"] {
            assert!(
                read_only
                    .execute(name, json!({"path":"x","content":"x","command":"true"}))
                    .await
                    .is_err()
            );
        }
        std::fs::write(
            root.path().join("large"),
            format!("HEAD:{}:TAIL", "x".repeat(MAX_BYTES)),
        )
        .unwrap();
        std::fs::write(root.path().join("binary"), [0xff]).unwrap();
        let large = read_only
            .execute("file_read", json!({"path":"large"}))
            .await
            .unwrap();
        assert!(large.len() <= MAX_BYTES);
        assert!(large.starts_with("HEAD:"));
        assert!(large.ends_with(":TAIL"));
        assert!(large.contains("[truncated]"));
        assert!(
            read_only
                .execute("file_read", json!({"path":"binary"}))
                .await
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        let write = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            write
                .execute(
                    "file_write",
                    json!({"path":"x","content":"x".repeat(MAX_BYTES+1)})
                )
                .await
                .is_err()
        );
        assert!(!root.path().join("x").exists());
        assert!(ToolHost::new(&root.path().join("does-not-exist"), &Config::default()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_cannot_escape_or_alias_protected_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "outside").unwrap();
        std::fs::create_dir(root.path().join(".kuru")).unwrap();
        std::fs::write(root.path().join(".kuru/memory"), "private").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        symlink(root.path().join(".kuru/memory"), root.path().join("alias")).unwrap();
        std::fs::hard_link(outside.path().join("secret"), root.path().join("hardlink")).unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_write: true,
                ..Default::default()
            },
        )
        .unwrap();
        let host = with_test_checkpoints(host, &private);
        for path in ["escape/secret", "alias", "hardlink"] {
            assert!(
                host.execute("file_read", json!({"path":path}))
                    .await
                    .is_err()
            );
        }
        for path in ["escape/secret", "alias"] {
            assert!(
                host.execute("file_write", json!({"path":path,"content":"bad"}))
                    .await
                    .is_err()
            );
        }
        // A multiply linked source cannot be selected for a checkpointed
        // replacement; neither the project link nor its other name changes.
        assert!(
            host.execute("file_write", json!({"path":"hardlink","content":"local"}))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("hardlink")).unwrap(),
            "outside"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "outside"
        );
        let listed = host.execute("file_list", json!({})).await.unwrap();
        assert!(!listed.contains("escape") && !listed.contains("alias"));
    }

    #[tokio::test]
    async fn shell_returns_status_bounds_output_and_terminates_on_timeout() {
        #[cfg(unix)]
        let (command, stall, flood, stderr_flood) = (
            "printf hello; printf problem >&2; exit 7",
            "sleep 5",
            "head -c 2097153 /dev/zero | tr '\\0' x",
            "head -c 2097153 /dev/zero | tr '\\0' x >&2",
        );
        #[cfg(windows)]
        let (command, stall, flood, stderr_flood) = (
            "[Console]::Out.Write('hello'); [Console]::Error.Write('problem'); exit 7",
            "Start-Sleep -Seconds 5",
            "[Console]::Out.Write('x' * 2097153)",
            "[Console]::Error.Write('x' * 2097153)",
        );
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|spec| spec.name == "shell")
        );
        let output: Value = serde_json::from_str(
            &host
                .execute("shell", json!({"command":command}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(output["stdout"], "hello");
        assert_eq!(output["stderr"], "problem");
        assert_eq!(output["exit_code"], 7);
        assert_eq!(output["success"], false);
        assert!(
            host.execute("shell", json!({"command":stall,"timeout_ms":20}))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        for invalid in [json!(0), json!(120001), json!("1")] {
            assert!(
                host.execute("shell", json!({"command":"true","timeout_ms":invalid}))
                    .await
                    .is_err()
            );
        }
        assert!(host.execute("shell", json!({"command":""})).await.is_err());
        for (command, stream, head, tail) in [
            (flood, "stdout", "x", "x"),
            (stderr_flood, "stderr", "x", "x"),
        ] {
            let output: Value = serde_json::from_str(
                &host
                    .execute("shell", json!({"command":command}))
                    .await
                    .unwrap(),
            )
            .unwrap();
            let retained = output[stream].as_str().unwrap();
            assert!(output["success"] == true);
            assert!(retained.len() <= MAX_BYTES);
            assert!(retained.starts_with(head));
            assert!(retained.ends_with(tail));
            assert!(retained.contains("[truncated]"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_keeps_independent_near_limit_stdout_and_stderr() {
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let output = host
            .execute(
                "shell",
                json!({
                    "command": "head -c 2097152 /dev/zero | tr '\\0' o; head -c 2097152 /dev/zero | tr '\\0' e >&2"
                }),
            )
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["stdout"], "o".repeat(MAX_BYTES));
        assert_eq!(output["stderr"], "e".repeat(MAX_BYTES));
        assert!(output["success"] == true);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_projection_redacts_both_streams_without_changing_exit_status() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let output = host
            .execute(
                "shell",
                json!({"command":format!("printf 'out {SECRET}'; printf 'err {SECRET}' >&2; exit 7")}),
            )
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["stdout"], "out [REDACTED:recognized-secret]");
        assert_eq!(output["stderr"], "err [REDACTED:recognized-secret]");
        assert_eq!(output["exit_code"], 7);
        assert!(output["success"] == false);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_timeout_projects_a_fixed_failure_without_captured_stderr() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("timeout-ready");
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let command = format!("printf '%s' '{SECRET}' >&2; : > timeout-ready; exec sleep 5");
        let (ready_result, call_result) = tokio::join!(
            timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }),
            timeout(
                Duration::from_secs(10),
                host.execute(
                    "shell",
                    json!({
                        "command": command,
                        "timeout_ms": 3_000,
                    }),
                ),
            )
        );
        let shutdown_result = timeout(Duration::from_secs(6), host.shutdown()).await;

        assert!(
            matches!(shutdown_result, Ok(Ok(()))),
            "timed shell shutdown did not finish"
        );
        ready_result.expect("timed shell did not reach readiness before the bounded wait");
        let error = call_result
            .expect("timed shell did not return within its timeout and cleanup allowance")
            .unwrap_err();
        let rendered = [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ];
        let chain_length = error.chain().count();

        assert_eq!(
            rendered[0],
            "tool execution failed: shell timed out; stderr: <pending EOF>"
        );
        for output in rendered {
            assert!(!output.contains(SECRET));
            assert!(!output.contains(&command));
            assert!(!output.contains("[REDACTED:recognized-secret]"));
        }
        assert_eq!(chain_length, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_timeout_keeps_eof_complete_redacted_stderr() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("stderr-closed");
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let command = format!(
            "printf 'useful {SECRET} detail' >&2; exec 2>&-; : > stderr-closed; exec sleep 5"
        );
        let (ready_result, call_result) = tokio::join!(
            timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }),
            timeout(
                Duration::from_secs(10),
                host.execute("shell", json!({"command": command, "timeout_ms": 3_000}),),
            )
        );
        let shutdown_result = timeout(Duration::from_secs(6), host.shutdown()).await;

        ready_result.expect("shell did not close stderr before timeout");
        assert!(matches!(shutdown_result, Ok(Ok(()))), "{shutdown_result:?}");
        let error = call_result
            .expect("timed shell did not return within its timeout and cleanup allowance")
            .unwrap_err();
        let rendered = [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ];
        for output in rendered {
            assert!(
                output.contains(
                    "shell timed out; stderr: useful [REDACTED:recognized-secret] detail"
                )
            );
            assert!(!output.contains(SECRET));
            assert!(!output.contains(&command));
            assert!(!output.contains("<pending EOF>"));
            assert!(!output.contains("<unavailable>"));
        }
        assert_eq!(error.chain().count(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "subprocess entrypoint for windows_shell_timeout_keeps_eof_complete_redacted_stderr"]
    async fn windows_shell_eof_launcher_retains_only_stdout() -> Result<()> {
        use kuru_platform::windows::process::{
            Lifetime, NativeSpawnSpec, StandardStream, Stdio, inherited_stdio, system_directory,
        };

        const ROOT: &str = "KURU_WINDOWS_SHELL_EOF_ROOT";
        let root = PathBuf::from(std::env::var_os(ROOT).context("missing test root")?);
        let system = system_directory()?;
        let mut spec = NativeSpawnSpec::new(system.join("ping.exe"), root.clone());
        spec.args = vec!["-n".into(), "90".into(), "127.0.0.1".into()];
        // This child stays in the outer ToolHost-owned Job after this short-lived
        // launcher exits. It must retain stdout only: the explicit native handle
        // list deliberately excludes the ToolHost stderr pipe.
        spec.lifetime = Lifetime::TrustedSupervisor;
        spec.stdout = inherited_stdio(StandardStream::Output)?;
        spec.stdin = Stdio::Null;
        spec.stderr = Stdio::Null;
        for key in ["SystemRoot", "WINDIR", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(key) {
                spec.environment.push((key.into(), value));
            }
        }
        let child = spec.spawn().await?;
        drop(child);
        std::fs::write(root.join("stderr-eof-ready"), "ready")?;
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_shell_timeout_keeps_eof_complete_redacted_stderr() {
        const SECRET: &str = "sk-abcdefghijklmnop";
        const OPERATION_TIMEOUT_SECS: u64 = 60;
        const READINESS_TIMEOUT: Duration = Duration::from_secs(OPERATION_TIMEOUT_SECS + 10);
        const OUTER_TIMEOUT: Duration = Duration::from_secs(OPERATION_TIMEOUT_SECS + 15);
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("stderr-eof-ready");
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let powershell_literal =
            |value: &OsStr| format!("'{}'", value.to_string_lossy().replace('\'', "''"));
        let root_literal = powershell_literal(root.path().as_os_str());
        let test_binary_literal = powershell_literal(
            std::env::current_exe()
                .expect("current test binary path")
                .as_os_str(),
        );
        // Instrumented test runs need the runner-selected profile destination
        // through this short-lived test-binary launcher.
        let llvm_profile = std::env::var_os("LLVM_PROFILE_FILE")
            .as_deref()
            .map(|value| format!("$env:LLVM_PROFILE_FILE = {}\n", powershell_literal(value)))
            .unwrap_or_default();
        let command = format!(
            r#"
$env:KURU_WINDOWS_SHELL_EOF_ROOT = {root_literal}
{llvm_profile}$launcher_info = New-Object System.Diagnostics.ProcessStartInfo
$launcher_info.FileName = {test_binary_literal}
$launcher_info.Arguments = '--exact tools::tests::windows_shell_eof_launcher_retains_only_stdout --ignored --nocapture'
$launcher_info.UseShellExecute = $false
$launcher_info.RedirectStandardOutput = $false
$launcher_info.RedirectStandardError = $false
$launcher = New-Object System.Diagnostics.Process
$launcher.StartInfo = $launcher_info
if (-not $launcher.Start()) {{ throw 'could not start stdout-retaining fixture launcher' }}
$launcher.WaitForExit()
if ($launcher.ExitCode -ne 0) {{ throw 'stdout-retaining fixture launcher failed' }}
[Console]::Error.Write('useful {SECRET} detail')
"#
        );
        let (ready_result, call_result) = tokio::join!(
            timeout(READINESS_TIMEOUT, async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }),
            timeout(
                OUTER_TIMEOUT,
                host.execute(
                    "shell",
                    json!({
                        "command": command,
                        "timeout_ms": OPERATION_TIMEOUT_SECS * 1_000,
                    }),
                ),
            )
        );
        ready_result.expect("Windows shell did not launch the stderr-isolated child");
        let error = call_result
            .expect("timed Windows shell did not return within native cleanup allowance")
            .unwrap_err();
        let rendered = [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            error
                .chain()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        ];
        for output in rendered {
            assert!(
                output.contains(
                    "shell timed out; stderr: useful [REDACTED:recognized-secret] detail"
                ),
                "{output}"
            );
            assert!(!output.contains(SECRET));
            assert!(!output.contains(&command));
            assert!(!output.contains("<pending EOF>"));
            assert!(!output.contains("<unavailable>"));
            assert!(!output.contains("cleanup: unconfirmed ownership retained"));
        }
        assert_eq!(error.chain().count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_requires_both_eof_and_root_exit_then_reaps_descendants() {
        let root = tempfile::tempdir().unwrap();
        let host = ToolHost::new(
            root.path(),
            &Config {
                allow_shell: true,
                ..Default::default()
            },
        )
        .unwrap();
        let started = std::time::Instant::now();
        let output: Value = serde_json::from_str(
            &host
                .execute("shell", json!({"command":"exec 1>&- 2>&-; sleep 0.15"}))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert_eq!(output["exit_code"], 0);
        assert_eq!(output["stdout"], "");
        assert_eq!(output["stderr"], "");

        let output: Value = serde_json::from_str(
            &host
                .execute(
                    "shell",
                    json!({"command":"sleep 5 </dev/null >/dev/null 2>/dev/null & exit 7"}),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(output["exit_code"], 7);
        assert_eq!(output["success"], false);
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shell_caller_loss_keeps_registered_owner_for_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("caller-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let call = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > caller-ready; exec sleep 5", "timeout_ms":120_000}),
                )
                .await
            }
        });
        timeout(Duration::from_secs(2), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shell caller did not reach readiness");
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        timeout(Duration::from_secs(6), host.shutdown())
            .await
            .expect("registered shell owner did not finish bounded shutdown")
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_shutdown_cancels_starting_and_active_shells_and_closes_mcp() {
        let peer = StdioFixture::new([
            Step::Read,
            Step::Write(
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}}),
            ),
            Step::Read,
            Step::Read,
            Step::Write(json!({"jsonrpc":"2.0","id":2,"result":{"tools":[]}})),
            Step::Eof,
        ]);
        let root = tempfile::tempdir().unwrap();
        let active_ready = root.path().join("active-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    mcp: [(
                        "fixture".into(),
                        McpConfig {
                            command: Some(peer.command().into()),
                            args: vec![],
                            url: None,
                            env: BTreeMap::new(),
                            ..Default::default()
                        },
                    )]
                    .into(),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        assert!(
            host.specs()
                .await
                .unwrap()
                .iter()
                .any(|spec| spec.name == "shell")
        );
        let starting_gate = host.test_shells().test_arm_start_gate();
        let starting = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > started-after-shutdown", "timeout_ms":120_000}),
                )
                .await
            }
        });
        timeout(Duration::from_secs(1), async {
            while host.test_shells().test_owner_count() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("starting shell was not registered");
        timeout(Duration::from_secs(1), async {
            while !starting_gate.entered() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("starting shell did not reach its controlled gate");
        let active = tokio::spawn({
            let host = host.clone();
            async move {
                host.execute(
                    "shell",
                    json!({"command":": > active-ready; exec sleep 5", "timeout_ms":120_000}),
                )
                .await
            }
        });
        let active_ready_result = timeout(Duration::from_secs(2), async {
            while !active_ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if active_ready_result.is_err() {
            let active_result = timeout(Duration::from_millis(100), active).await;
            starting_gate.release();
            let _ = host.shutdown().await;
            panic!("active shell did not reach readiness: {active_result:?}");
        }
        let shutdown = tokio::spawn({
            let host = host.clone();
            async move { host.shutdown().await }
        });
        timeout(Duration::from_secs(1), async {
            while !host.test_shells().test_is_closing() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shutdown did not close shell registration");
        let rejected = host
            .execute("shell", json!({"command":": > launched-after-shutdown"}))
            .await
            .unwrap_err();
        assert_eq!(
            rejected.to_string(),
            "tool execution failed: shell operation failed; stderr: <pending EOF>"
        );
        starting_gate.release();
        timeout(Duration::from_secs(6), shutdown)
            .await
            .expect("combined shell/MCP shutdown did not finish")
            .unwrap()
            .unwrap();
        assert!(
            starting
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert!(
            active
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        peer.assert_completed(1);
        assert!(!root.path().join("started-after-shutdown").exists());
        assert!(!root.path().join("launched-after-shutdown").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_outlives_a_destroyed_parent_runtime() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("worker-ready");
        let host = Arc::new(
            ToolHost::new(
                root.path(),
                &Config {
                    allow_shell: true,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let root = root.path().to_path_buf();
            runtime.block_on(async {
                let _call = tokio::spawn({
                    let host = host.clone();
                    async move {
                        host.execute(
                            "shell",
                            json!({"command":": > worker-ready; exec sleep 5", "timeout_ms":120_000}),
                        )
                        .await
                    }
                });
                timeout(Duration::from_secs(2), async {
                    while !root.join("worker-ready").exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("shell worker did not reach readiness before runtime destruction");
            });
        }
        assert!(ready.exists());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            timeout(Duration::from_secs(6), host.shutdown())
                .await
                .expect("retained worker did not finish after parent runtime destruction")
                .unwrap();
        });
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retained_root_replacement_refuses_all_tool_dispatch() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("held.txt"), "held object").unwrap();
        let retained =
            Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Movable).unwrap());
        let host = ToolHost::with_retained_root(
            retained,
            &Config {
                allow_shell: true,
                ..Config::default()
            },
        )
        .unwrap();
        std::fs::rename(&root, parent.path().join("replaced")).unwrap();
        std::fs::create_dir(&root).unwrap();

        // A held directory can still name the moved file, but permission
        // context is bound to the reviewed root name and identity. A
        // replacement invalidates that context before every tool effect.
        let file_error = host
            .execute("file_read", json!({"path":"held.txt"}))
            .await
            .unwrap_err();
        let shell_error = host
            .execute(
                "shell",
                json!({"command": if cfg!(windows) { "[IO.File]::WriteAllText('launched', 'started')" } else { "printf started > launched" }}),
            )
            .await
            .unwrap_err();
        let search_error = host
            .execute("glob", json!({"pattern":"**/*"}))
            .await
            .unwrap_err();
        for error in [&file_error, &shell_error, &search_error] {
            assert_eq!(
                error.to_string(),
                "tool execution failed: directory name or ancestor identity changed"
            );
            assert_eq!(format!("{error:#}"), error.to_string());
        }
        assert_eq!(
            std::fs::read_to_string(parent.path().join("replaced/held.txt")).unwrap(),
            "held object"
        );
        assert!(!root.join("launched").exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn retained_root_handle_prevents_substitution_before_tool_dispatch() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("held.txt"), "held object").unwrap();
        let retained =
            Arc::new(Directory::open(&root, Privacy::Inherited, NameRetention::Movable).unwrap());
        let original_identity = retained.identity();
        let host = ToolHost::with_retained_root(retained, &Config::default()).unwrap();

        // cap-std retains a second Windows directory handle without
        // FILE_SHARE_DELETE. The attempted substitution is rejected by the OS
        // before a replacement root can grant any tool effect.
        let error = std::fs::rename(&root, parent.path().join("replaced")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32));
        assert!(!parent.path().join("replaced").exists());
        assert_eq!(
            Directory::open(&root, Privacy::Inherited, NameRetention::Movable)
                .unwrap()
                .identity(),
            original_identity
        );
        assert_eq!(
            host.execute("file_read", json!({"path":"held.txt"}))
                .await
                .unwrap(),
            "held object"
        );
        assert!(!root.join("launched").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_environment_has_an_exact_independent_inventory() {
        use std::os::unix::ffi::OsStringExt;

        let expected = vec![
            ("PATH".into(), "/fixture/bin".into()),
            ("HOME".into(), "/fixture/home".into()),
            ("USER".into(), "fixture-user".into()),
            ("LOGNAME".into(), "fixture-login".into()),
            ("TMPDIR".into(), "/fixture/tmpdir".into()),
            ("TMP".into(), "/fixture/tmp".into()),
            ("TEMP".into(), "/fixture/temp".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LC_COLLATE".into(), "C".into()),
            ("LC_CTYPE".into(), OsString::from_vec(vec![b'x', 0xff])),
            ("LC_MESSAGES".into(), "C".into()),
            ("LC_MONETARY".into(), "C".into()),
            ("LC_NUMERIC".into(), "C".into()),
            ("LC_TIME".into(), "C".into()),
            ("TZ".into(), "UTC".into()),
            ("NO_COLOR".into(), "1".into()),
            ("XDG_CONFIG_HOME".into(), "/fixture/config".into()),
            ("XDG_CACHE_HOME".into(), "/fixture/cache".into()),
            ("XDG_DATA_HOME".into(), "/fixture/data".into()),
            ("XDG_STATE_HOME".into(), "/fixture/state".into()),
            ("XDG_RUNTIME_DIR".into(), "/fixture/runtime".into()),
        ];
        let mut input = expected.clone();
        input.extend([
            ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ("openai_api_key".into(), "also-ignored".into()),
            ("HTTP_PROXY".into(), "fake-proxy".into()),
            ("SSH_AUTH_SOCK".into(), "fake-agent".into()),
            ("GIT_ASKPASS".into(), "fake-askpass".into()),
            ("LD_LIBRARY_PATH".into(), "/fixture/loader".into()),
            ("BASH_ENV".into(), "fake-startup".into()),
            ("KURU_TEST_SENTINEL".into(), "fake-kuru".into()),
            ("UNRELATED_SHELL_FIXTURE".into(), "ignored".into()),
            (OsString::from_vec(vec![0xff]), "nonunicode-key".into()),
        ]);
        let projected = unix_shell_environment(input);

        assert_eq!(projected, expected);
        assert_eq!(projected[10].1.clone().into_vec(), vec![b'x', 0xff]);
        assert_eq!(
            unix_shell_environment([
                ("HOME".into(), "/fixture/home".into()),
                ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ]),
            vec![(OsString::from("HOME"), OsString::from("/fixture/home"))]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_environment_has_an_exact_independent_inventory() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let system = Path::new(r"C:\Windows\System32");
        let expected = vec![
            ("PATH".into(), r"C:\fixture\bin".into()),
            ("HOME".into(), r"C:\fixture\home".into()),
            ("USER".into(), "fixture-user".into()),
            ("LOGNAME".into(), "fixture-login".into()),
            ("USERNAME".into(), "fixture-name".into()),
            ("USERPROFILE".into(), r"C:\fixture\profile".into()),
            ("HOMEDRIVE".into(), "C:".into()),
            ("HOMEPATH".into(), r"\fixture\profile".into()),
            ("APPDATA".into(), r"C:\fixture\appdata".into()),
            ("LOCALAPPDATA".into(), r"C:\fixture\localappdata".into()),
            ("ProgramData".into(), r"C:\fixture\programdata".into()),
            ("ProgramFiles".into(), r"C:\fixture\programfiles".into()),
            (
                "ProgramFiles(x86)".into(),
                r"C:\fixture\programfiles-x86".into(),
            ),
            ("ProgramW6432".into(), r"C:\fixture\programfiles-64".into()),
            ("PROCESSOR_ARCHITECTURE".into(), "AMD64".into()),
            ("PROCESSOR_ARCHITEW6432".into(), "AMD64".into()),
            ("TMPDIR".into(), r"C:\fixture\tmpdir".into()),
            ("TMP".into(), r"C:\fixture\tmp".into()),
            ("TEMP".into(), r"C:\fixture\temp".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LC_COLLATE".into(), "C".into()),
            (
                "LC_CTYPE".into(),
                OsString::from_wide(&[0xd800, b'x' as u16]),
            ),
            ("LC_MESSAGES".into(), "C".into()),
            ("LC_MONETARY".into(), "C".into()),
            ("LC_NUMERIC".into(), "C".into()),
            ("LC_TIME".into(), "C".into()),
            ("TZ".into(), "UTC".into()),
            ("NO_COLOR".into(), "1".into()),
            ("XDG_CONFIG_HOME".into(), r"C:\fixture\config".into()),
            ("XDG_CACHE_HOME".into(), r"C:\fixture\cache".into()),
            ("XDG_DATA_HOME".into(), r"C:\fixture\data".into()),
            ("XDG_STATE_HOME".into(), r"C:\fixture\state".into()),
            ("XDG_RUNTIME_DIR".into(), r"C:\fixture\runtime".into()),
            ("PATHEXT".into(), ".EXE;.CMD".into()),
            ("SystemRoot".into(), r"C:\Windows".into()),
            ("WINDIR".into(), r"C:\Windows".into()),
            ("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into()),
        ];
        let mut input: Vec<(OsString, OsString)> = expected[..35].to_vec();
        input[0].0 = "pAtH".into();
        input[5].0 = "userprofile".into();
        input[34].0 = "pAtHeXt".into();
        input.extend([
            ("PSMODULEPATH".into(), "hostile-one".into()),
            ("PsModulePath".into(), "hostile-two".into()),
            ("SystemRoot".into(), r"C:\hostile-one".into()),
            ("SYSTEMROOT".into(), r"C:\hostile-two".into()),
            ("WINDIR".into(), r"C:\hostile-windir-one".into()),
            ("windir".into(), r"C:\hostile-windir-two".into()),
            ("ComSpec".into(), r"C:\hostile-cmd-one.exe".into()),
            ("COMSPEC".into(), r"C:\hostile-cmd-two.exe".into()),
            ("OPENAI_API_KEY".into(), "fake-secret".into()),
            ("openai_api_key".into(), "also-ignored".into()),
            ("KURU_TEST_SENTINEL".into(), "fake-kuru".into()),
        ]);
        let projected = windows_shell_environment(input, system).unwrap();

        assert_eq!(projected, expected);
        assert_eq!(
            projected[22].1.encode_wide().collect::<Vec<_>>(),
            [0xd800, b'x' as u16]
        );
        assert_eq!(
            windows_shell_environment([("HOME".into(), r"C:\fixture\home".into())], system)
                .unwrap(),
            vec![
                (OsString::from("HOME"), OsString::from(r"C:\fixture\home")),
                (
                    OsString::from("PATHEXT"),
                    OsString::from(".COM;.EXE;.BAT;.CMD")
                ),
                (OsString::from("SystemRoot"), OsString::from(r"C:\Windows")),
                (OsString::from("WINDIR"), OsString::from(r"C:\Windows")),
                (
                    OsString::from("ComSpec"),
                    OsString::from(r"C:\Windows\System32\cmd.exe")
                ),
            ]
        );
        assert!(
            windows_shell_environment(
                [("Path".into(), "one".into()), ("PATH".into(), "two".into())],
                system,
            )
            .is_err()
        );
        assert!(
            windows_shell_environment(
                [
                    ("pAtHeXt".into(), ".EXE".into()),
                    ("PATHEXT".into(), ".CMD".into()),
                ],
                system,
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    fn windows_shell_projection_source(root: &Path, case: &str) -> String {
        let home = root.join("home");
        let temporary = root.join("temporary");
        let commands = root.join("commands");
        let system = kuru_platform::windows::process::system_directory().unwrap();
        let windows = system.parent().unwrap();
        let expected_path = std::env::join_paths([commands.as_path(), system.as_path()])
            .expect("fixture paths must form a Windows PATH");
        let powershell_literal =
            |value: &OsStr| format!("'{}'", value.to_string_lossy().replace('\'', "''"));
        let expected_path = powershell_literal(&expected_path);
        let expected_commands = powershell_literal(commands.as_os_str());
        let expected_home = powershell_literal(home.as_os_str());
        let expected_temporary = powershell_literal(temporary.as_os_str());
        let expected_windows = powershell_literal(windows.as_os_str());
        let expected_comspec = powershell_literal(system.join("cmd.exe").as_os_str());
        let expected_stage = powershell_literal(
            temporary
                .join(format!("shell-stage-{case}.txt"))
                .as_os_str(),
        );
        format!(
            r#"
[IO.File]::AppendAllText({expected_stage}, "entered`n")
$stage = {expected_stage}
$expectedManagementModule = [IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Management\Microsoft.PowerShell.Management.psd1')
$expectedUtilityModule = [IO.Path]::Combine($PSHOME, 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1')
$managementModules = @(Microsoft.PowerShell.Core\Get-Module -Name 'Microsoft.PowerShell.Management')
$utilityModules = @(Microsoft.PowerShell.Core\Get-Module -Name 'Microsoft.PowerShell.Utility')
$managementLoaded = $managementModules.Count -eq 1 -and [string]::Equals($managementModules[0].Path, $expectedManagementModule, [System.StringComparison]::OrdinalIgnoreCase)
$utilityLoaded = $utilityModules.Count -eq 1 -and [string]::Equals($utilityModules[0].Path, $expectedUtilityModule, [System.StringComparison]::OrdinalIgnoreCase)
if (-not $managementLoaded -or -not $utilityLoaded) {{ throw 'stock shell module bootstrap did not load the exact PSHOME manifests' }}
[IO.File]::AppendAllText($stage, "stock-modules-loaded`n")
$expectedPath = {expected_path}
$expectedCommands = {expected_commands}
$expectedHome = {expected_home}
$expectedTemporary = {expected_temporary}
$expectedWindows = {expected_windows}
$expectedComSpec = {expected_comspec}
# Kuru supplies the exact inherited value or fallback. Stock PowerShell then
# appends .CPL during engine construction when that extension is absent.
$expectedPathext = if ($env:NO_COLOR -eq 'inherited') {{ '.EXE;.CMD;.CPL' }} else {{ '.COM;.EXE;.BAT;.CMD;.CPL' }}
[IO.File]::AppendAllText($stage, "before-join-path`n")
$homePath = Join-Path $env:USERPROFILE 'shell-home.txt'
[IO.File]::AppendAllText($stage, "after-join-path`n")
[IO.File]::WriteAllText($homePath, 'home')
$temporaryPath = Join-Path $env:TEMP 'shell-temp.txt'
[IO.File]::WriteAllText($temporaryPath, 'temp')
$probeHash = Get-FileHash -LiteralPath ([IO.Path]::Combine($expectedCommands, 'probe.cmd')) -Algorithm SHA256
[IO.File]::AppendAllText($stage, "home-temp-written`n")
[IO.File]::AppendAllText($stage, "before-where`n")
$where = & where.exe cmd.exe
$whereOk = $LASTEXITCODE -eq 0
[IO.File]::AppendAllText($stage, "after-where`n")
[IO.File]::AppendAllText($stage, "before-probe`n")
$probe = & probe
[IO.File]::AppendAllText($stage, "after-probe`n")
[IO.File]::AppendAllText($stage, "before-checks`n")
$checks = [ordered]@{{
    PATH = [string]::Equals($env:PATH, $expectedPath, [System.StringComparison]::Ordinal)
    HOME = [string]::Equals($env:HOME, $expectedHome, [System.StringComparison]::Ordinal)
    USERPROFILE = [string]::Equals($env:USERPROFILE, $expectedHome, [System.StringComparison]::Ordinal)
    TEMP = [string]::Equals($env:TEMP, $expectedTemporary, [System.StringComparison]::Ordinal)
    PATHEXT = [string]::Equals($env:PATHEXT, $expectedPathext, [System.StringComparison]::Ordinal)
    SystemRoot = [string]::Equals($env:SystemRoot, $expectedWindows, [System.StringComparison]::Ordinal)
    WINDIR = [string]::Equals($env:WINDIR, $expectedWindows, [System.StringComparison]::Ordinal)
    ComSpec = [string]::Equals($env:ComSpec, $expectedComSpec, [System.StringComparison]::Ordinal)
    OPENAI_API_KEY_absent = -not (Test-Path Env:OPENAI_API_KEY)
    HTTP_PROXY_absent = -not (Test-Path Env:HTTP_PROXY)
    PSModulePath_reconstructed = -not [string]::IsNullOrEmpty($env:PSModulePath)
    PSModulePath_hostile_absent = $env:PSModulePath -notlike '*fake-modules*'
    fixture_child_absent = -not (Test-Path Env:KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD)
    LLVM_PROFILE_FILE_absent = -not (Test-Path Env:LLVM_PROFILE_FILE)
    join_path_source = (Get-Command Join-Path -ErrorAction Stop).ModuleName -eq 'Microsoft.PowerShell.Management'
    stock_hash = $probeHash.Hash -eq '87FC3ECEAA19AC72D1A5B575708934E4A31DAEFC465A8A26CF01E4FC38D6B018'
    file_hash_source = (Get-Command Get-FileHash -ErrorAction Stop).ModuleName -eq 'Microsoft.PowerShell.Utility'
    stock_cmdlet = (Get-Command Get-ChildItem -ErrorAction Stop).CommandType -eq 'Cmdlet'
    where_cmd = $whereOk
    probe_cmd = $probe -contains 'cmd-ok'
}}
[IO.File]::AppendAllText($stage, "after-checks`n")
$failed = @()
foreach ($check in $checks.GetEnumerator()) {{
    if (-not $check.Value) {{ $failed += [string]$check.Key }}
}}
if ($failed.Count -eq 0) {{
    [Console]::Out.Write('ok')
}} else {{
    throw ('shell compatibility fixture conditions failed: ' + [string]::Join(',', $failed))
}}
"#
        )
    }

    #[cfg(windows)]
    fn windows_shell_projection_stages(root: &Path, case: &str) -> String {
        let stage = root
            .join("temporary")
            .join(format!("shell-stage-{case}.txt"));
        let mut bytes = Vec::new();
        match std::fs::File::open(stage) {
            Ok(file) => {
                let mut limited = std::io::Read::take(file, 4096);
                match std::io::Read::read_to_end(&mut limited, &mut bytes) {
                    Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(error) => format!("<unreadable: {error}>"),
                }
            }
            Err(error) => format!("<unavailable: {error}>"),
        }
    }

    #[cfg(windows)]
    const WINDOWS_SHELL_FIXTURE_CHILD: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD";

    fn windows_shell_cleanup_uncertain(stdout: &[u8], stderr: &[u8]) -> bool {
        let marker = b"subprocess cleanup unconfirmed";
        stdout.windows(marker.len()).any(|bytes| bytes == marker)
            || stderr.windows(marker.len()).any(|bytes| bytes == marker)
    }

    #[test]
    fn windows_shell_cleanup_uncertainty_marker_is_detected() {
        assert!(windows_shell_cleanup_uncertain(
            b"prefix subprocess cleanup unconfirmed suffix",
            b""
        ));
        assert!(windows_shell_cleanup_uncertain(
            b"",
            b"prefix subprocess cleanup unconfirmed suffix"
        ));
        assert!(!windows_shell_cleanup_uncertain(
            b"subprocess tree terminated",
            b"ordinary failure"
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_toolhost_shell_receives_only_compatibility_environment() {
        const CHILD: &str = "KURU_SHELL_ENVIRONMENT_TEST_CHILD";
        const ROOT: &str = "KURU_SHELL_ENVIRONMENT_TEST_ROOT";
        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("missing test root"));
            let home = root.join("home");
            let temporary = root.join("temporary");
            let cache = root.join("cache");
            let quote = |path: &Path| {
                format!(
                    "'{}'",
                    path.as_os_str().to_string_lossy().replace('\'', "'\"'\"'")
                )
            };
            let expected_home = quote(&home);
            let expected_temporary = quote(&temporary);
            let expected_cache = quote(&cache);
            let shell = [
                "test \"$PATH\" = '/usr/bin:/bin' && test \"$HOME\" = ",
                &expected_home,
                " && test \"$TMPDIR\" = ",
                &expected_temporary,
                " && test \"$XDG_CACHE_HOME\" = ",
                &expected_cache,
                " && test \"$LC_CTYPE\" = 'C.UTF-8' && test -z \"${OPENAI_API_KEY+x}\" && test -z \"${HTTP_PROXY+x}\" && test -z \"${SSH_AUTH_SOCK+x}\" && test -z \"${GIT_ASKPASS+x}\" && test -z \"${LD_LIBRARY_PATH+x}\" && test -z \"${BASH_ENV+x}\" && test -z \"${KURU_SHELL_ENVIRONMENT_TEST_CHILD+x}\" && test -z \"${UNRELATED_SHELL_FIXTURE+x}\" && test -z \"${LLVM_PROFILE_FILE+x}\" && touch cwd-write \"$HOME/home-write\" \"$TMPDIR/temp-write\" && test -f cwd-write && test -f \"$HOME/home-write\" && test -f \"$TMPDIR/temp-write\" && printf ok",
            ]
            .concat();
            let host = ToolHost::new(
                &root,
                &Config {
                    allow_shell: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let output: Value = serde_json::from_str(
                &host
                    .execute("shell", json!({"command":shell}))
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(
                output["success"] == true && output["stdout"] == "ok",
                "shell compatibility environment was not minimized"
            );
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let temporary = root.path().join("temporary");
        let cache = root.path().join("cache");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&temporary).unwrap();
        std::fs::create_dir(&cache).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("tools::tests::isolated_toolhost_shell_receives_only_compatibility_environment")
            .arg("--nocapture")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("TMPDIR", &temporary)
            .env("LC_CTYPE", "C.UTF-8")
            .env("XDG_CACHE_HOME", &cache)
            .env("OPENAI_API_KEY", "fake-api-key")
            .env("HTTP_PROXY", "fake-proxy")
            .env("SSH_AUTH_SOCK", "fake-agent")
            .env("GIT_ASKPASS", "fake-askpass")
            .env("LD_LIBRARY_PATH", "/fake/loader")
            .env("BASH_ENV", "fake-startup")
            .env("UNRELATED_SHELL_FIXTURE", "ignored")
            .env(CHILD, "1")
            .env(ROOT, root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut child = command.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let result = timeout(Duration::from_secs(10), async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                drain_bounded(&mut stdout, &mut out),
                drain_bounded(&mut stderr, &mut err),
                child.wait(),
            );
            if stdout_truncated? || stderr_truncated? {
                return Err(std::io::Error::other(
                    "isolated shell fixture output exceeds 2 MiB",
                ));
            }
            Ok::<_, std::io::Error>((status?, out, err))
        })
        .await;
        let (status, out, err) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                let cleanup_confirmed = stdout_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && stderr_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && reap.as_ref().is_ok_and(|result| result.is_ok());
                if !cleanup_confirmed {
                    let preserved = root.keep();
                    panic!(
                        "isolated shell environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}; preserved {}",
                        preserved.display()
                    );
                }
                panic!(
                    "isolated shell environment fixture failed: {error}; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}"
                );
            }
            Err(_) => {
                let requested = child.start_kill();
                let (stdout_drain, stderr_drain, reap) = tokio::join!(
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stdout, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), async {
                        let mut discarded = Vec::new();
                        drain_bounded(&mut stderr, &mut discarded).await
                    }),
                    timeout(Duration::from_secs(5), child.wait()),
                );
                let cleanup_confirmed = stdout_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && stderr_drain.as_ref().is_ok_and(|result| result.is_ok())
                    && reap.as_ref().is_ok_and(|result| result.is_ok());
                if !cleanup_confirmed {
                    let preserved = root.keep();
                    panic!(
                        "isolated shell environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}; reap={reap:?}; preserved {}",
                        preserved.display()
                    );
                }
                panic!(
                    "isolated shell environment fixture timed out; kill={requested:?}; stdout-drain={stdout_drain:?}; stderr-drain={stderr_drain:?}"
                );
            }
        };
        assert!(
            status.success(),
            "isolated shell environment fixture failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        assert!(root.path().join("cwd-write").is_file());
        assert!(home.join("home-write").is_file());
        assert!(temporary.join("temp-write").is_file());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn isolated_toolhost_windows_shell_receives_projected_environment() {
        use kuru_platform::windows::process::{NativeSpawnSpec, Stdio, system_directory};

        const CHILD: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_CHILD";
        const ROOT: &str = "KURU_WINDOWS_SHELL_ENVIRONMENT_TEST_ROOT";
        const OPERATION_TIMEOUT_SECS: u64 = 60;
        const CHILD_WAIT_TIMEOUT: Duration = Duration::from_secs(OPERATION_TIMEOUT_SECS + 10);
        const OUTER_TIMEOUT: Duration = Duration::from_secs(OPERATION_TIMEOUT_SECS + 15);

        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("missing test root"));
            let home = root.join("home");
            let temporary = root.join("temporary");
            let case = match std::env::var("NO_COLOR").as_deref() {
                Ok("inherited") => "inherited",
                Ok("fallback") => "fallback",
                value => panic!("unexpected Windows shell fixture case: {value:?}"),
            };
            let stage_trace = || windows_shell_projection_stages(&root, case);
            let host = ToolHost::new(
                &root,
                &Config {
                    allow_shell: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let shell = windows_shell_projection_source(&root, case);
            let receipt = host
                .execute(
                    "shell",
                    json!({"command":shell,"timeout_ms":OPERATION_TIMEOUT_SECS * 1_000}),
                )
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "Windows shell projection fixture failed: {error:#}; stages={}",
                        stage_trace()
                    )
                });
            let output: Value = serde_json::from_str(&receipt).unwrap_or_else(|error| {
                panic!(
                    "Windows shell projection fixture returned invalid receipt: {error}; receipt={receipt}; stages={}",
                    stage_trace()
                )
            });
            assert!(
                output["success"] == true && output["stdout"] == "ok",
                "Windows shell projection fixture failed: receipt={output}; stages={}",
                stage_trace()
            );
            assert_eq!(
                stage_trace(),
                "source-entered\nmanagement-imported\nutility-imported\nentered\nstock-modules-loaded\nbefore-join-path\nafter-join-path\nhome-temp-written\nbefore-where\nafter-where\nbefore-probe\nafter-probe\nbefore-checks\nafter-checks\n"
            );
            assert_eq!(
                std::fs::read_to_string(home.join("shell-home.txt")).unwrap(),
                "home"
            );
            assert_eq!(
                std::fs::read_to_string(temporary.join("shell-temp.txt")).unwrap(),
                "temp"
            );
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let commands = root.path().join("commands");
        let home = root.path().join("home");
        let temporary = root.path().join("temporary");
        let local = root.path().join("local");
        let roaming = root.path().join("roaming");
        std::fs::create_dir(&commands).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&temporary).unwrap();
        std::fs::create_dir(&local).unwrap();
        std::fs::create_dir(&roaming).unwrap();
        std::fs::write(commands.join("probe.cmd"), "@echo cmd-ok\r\n").unwrap();
        let system = system_directory().unwrap();
        let path = std::env::join_paths([commands.as_path(), system.as_path()]).unwrap();
        let mut failure = None;
        let mut preserve_root = false;
        for (name, pathext) in [("inherited", Some(".EXE;.CMD")), ("fallback", None)] {
            let mut spec =
                NativeSpawnSpec::new(std::env::current_exe().unwrap(), root.path().into());
            spec.args = vec![
                "--exact".into(),
                "tools::tests::isolated_toolhost_windows_shell_receives_projected_environment"
                    .into(),
                "--nocapture".into(),
            ];
            spec.environment = vec![
                ("pAtH".into(), path.clone()),
                ("HOME".into(), home.clone().into()),
                ("userprofile".into(), home.clone().into()),
                ("LOCALAPPDATA".into(), local.clone().into()),
                ("APPDATA".into(), roaming.clone().into()),
                ("TEMP".into(), temporary.clone().into()),
                ("TMP".into(), temporary.clone().into()),
                ("NO_COLOR".into(), name.into()),
                ("SystemRoot".into(), r"C:\hostile-system".into()),
                ("WINDIR".into(), r"C:\hostile-windir".into()),
                ("ComSpec".into(), r"C:\hostile-cmd.exe".into()),
                ("OPENAI_API_KEY".into(), "fake-api-key".into()),
                ("HTTP_PROXY".into(), "fake-proxy".into()),
                ("pSmOdUlEpAtH".into(), "fake-modules".into()),
                (CHILD.into(), "1".into()),
                (ROOT.into(), root.path().into()),
            ];
            for key in [
                "PROCESSOR_ARCHITECTURE",
                "PROCESSOR_ARCHITEW6432",
                "ProgramData",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    spec.environment.push((key.into(), value));
                }
            }
            if let Some(pathext) = pathext {
                spec.environment.push(("pAtHeXt".into(), pathext.into()));
            }
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
            }
            spec.stdout = Stdio::Pipe;
            spec.stderr = Stdio::Pipe;
            let mut child = spec.spawn().await.unwrap();
            let mut stdout = child.take_stdout().unwrap();
            let mut stderr = child.take_stderr().unwrap();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let outcome = timeout(OUTER_TIMEOUT, async {
                let (stdout_truncated, stderr_truncated, status) = tokio::join!(
                    drain_bounded(&mut stdout, &mut out),
                    drain_bounded(&mut stderr, &mut err),
                    child.wait(CHILD_WAIT_TIMEOUT),
                );
                let stdout_truncated = stdout_truncated?;
                let stderr_truncated = stderr_truncated?;
                if stdout_truncated || stderr_truncated {
                    return Err(std::io::Error::other(
                        "Windows shell fixture output exceeds 2 MiB",
                    ));
                }
                Ok::<_, std::io::Error>(status?)
            })
            .await;
            let status = match outcome {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(error)) => {
                    let requested = child.terminate();
                    let (stdout_close, stderr_close) = tokio::join!(
                        stdout.close(Duration::from_secs(5)),
                        stderr.close(Duration::from_secs(5)),
                    );
                    let reap = child.wait(Duration::from_secs(5)).await;
                    if stdout_close.is_err() || stderr_close.is_err() || reap.is_err() {
                        preserve_root = true;
                    }
                    Err(format!(
                        "Windows shell fixture {name} failed: {error}; kill={requested:?}; stdout-close={stdout_close:?}; stderr-close={stderr_close:?}; reap={reap:?}; stderr-prefix={}",
                        String::from_utf8_lossy(&err[..err.len().min(4096)])
                    ))
                }
                Err(_) => {
                    let requested = child.terminate();
                    let (stdout_close, stderr_close) = tokio::join!(
                        stdout.close(Duration::from_secs(5)),
                        stderr.close(Duration::from_secs(5)),
                    );
                    let reap = child.wait(Duration::from_secs(5)).await;
                    if stdout_close.is_err() || stderr_close.is_err() || reap.is_err() {
                        preserve_root = true;
                    }
                    Err(format!(
                        "Windows shell fixture {name} timed out; kill={requested:?}; stdout-close={stdout_close:?}; stderr-close={stderr_close:?}; reap={reap:?}; stderr-prefix={}",
                        String::from_utf8_lossy(&err[..err.len().min(4096)])
                    ))
                }
            };
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            if !status.success() {
                preserve_root |= windows_shell_cleanup_uncertain(&out, &err);
                failure = Some(format!(
                    "Windows shell fixture {name} failed: stdout={} stderr={}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                ));
                break;
            }
        }
        if let Some(failure) = failure {
            if preserve_root {
                let preserved = root.keep();
                panic!("{failure}; preserved {}", preserved.display());
            }
            panic!("{failure}");
        }
    }
}
