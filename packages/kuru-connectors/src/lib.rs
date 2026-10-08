//! Provider-neutral inference and explicit tool execution. No adapter owns actors
//! or persistent memory; the runtime supplies each actor's permitted context.

mod a2a;
mod auth;
mod compatibility;
mod file_edits;
mod hooks;
mod http;
mod instruction_review;
mod mcp;
mod mcp_cache;
mod mcp_credentials;
mod mcp_oauth;
pub mod permissions;
#[cfg(windows)]
mod process;
mod providers;
mod redaction;
mod retry;
mod rpc;
mod shell_diagnostic;
#[cfg(any(test, feature = "test-support"))]
pub mod shell_warmup;
mod tool_output;
mod tools;
#[cfg(unix)]
mod unix_shell;
mod web_fetch;
#[cfg(windows)]
mod windows_shell;

#[cfg(test)]
mod test_support;

pub use a2a::a2a_send;
pub use auth::{AuthManager, AuthStatus, BrowserLogin, DeviceLogin};
pub use compatibility::{CompatibilityCode, ConnectorIncompatibility, incompatibility};
pub use file_edits::{
    CheckpointDiff, CheckpointDiffReader, CheckpointState, CheckpointStore, CheckpointSummary,
    FileEffect,
};
pub use hooks::{
    HookAnnotation, HookBudget, HookHost, HookObservation, HookOutcomeKind,
    MAX_PRE_TURN_INPUT_BYTES, PostHookRun, PreHookOutcome, PreHookRun, PreToolValue, PreTurnValue,
    SpeakerHookOutcome, SpeakerHookRun,
};
/// Drop-only ownership captured for one invocation's retained native work.
/// It conveys no tool capability, session identity or completion evidence.
pub type InvocationHold = std::sync::Arc<dyn std::any::Any + Send + Sync>;
pub use instruction_review::{
    InstructionActivation, InstructionGate, InstructionGateOutcome, InstructionReviewAnswer,
    InstructionReviewRequest, InstructionReviewSender, SkillGate,
};
pub use mcp::{
    McpAvailability, McpBrowserLogin, McpDeviceLogin, McpOAuthAliasStatus, McpOAuthLogout,
    McpStatus,
};
pub use mcp_cache::McpCatalogStore;
pub use mcp_credentials::McpCredentialStore;
pub use permissions::{
    ApprovalAnswer, ApprovalRequest, ApprovalSender, GrantInspection, GrantRevocation, GrantScope,
    PermissionBinding, PermissionDisplay, PermissionGrantStore, PermissionInvocation,
    PermissionOutcome, PermissionService, PersistentGrant,
};
#[cfg(feature = "test-support")]
pub use providers::subscription_canary_test_endpoint;
pub use providers::{
    COMPLETION_TIMEOUT, CanaryReport, CanaryStage, CanaryState, ContextPrefixEstimate,
    DemoProvider, Provider, ProviderEvent, ProviderFailureKind, ProviderReasoningSummary,
    ProviderSink, ResponsesProvider, TextDeltaSource, collect_completion,
    discover_subscription_models, largest_fitting_context_prefix, provider, subscription_canary,
};
pub use redaction::{
    ProjectionError, json as project_json, text as project_text, truncate_tool_output,
};
pub use shell_diagnostic::{
    MAX_SHELL_PREVIEW_BYTES, ShellPreview, ShellPreviewSnapshot, ShellProgress, ShellStream,
};
pub use tool_output::is_permission_denied;
#[cfg(any(test, feature = "test-support"))]
pub use tools::ParallelReadTestGate;
pub use tools::{
    ActorToolOutcome, ParallelReadAdmission, ParallelReadCancellation, PreparedRead, ToolCatalog,
    ToolHost, ToolInvocationContext,
};

/// Maximum protocol message/body size; limits also apply to chunked responses.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Longest silence a streamed completion may hold between wire chunks before
/// the stream fails; any chunk, including a frame Kuru does not forward,
/// restarts it.
///
/// Derivation: the Responses streaming-events reference documents no
/// keepalive, heartbeat or inter-event bound for HTTP SSE (developers.openai.com
/// `api/reference/resources/responses/streaming-events`, read 2026-10-03), and
/// the background-mode guide states reasoning models "can take several minutes"
/// on complex problems, so silent reasoning between events is expected. The
/// vendor's own Responses client documents a 300000 ms default SSE idle timeout
/// (Codex configuration reference, `model_providers.<id>.stream_idle_timeout_ms`,
/// read 2026-10-03). Kuru adopts that client default as its product silence
/// budget; it is not a server guarantee. The total remains [`COMPLETION_TIMEOUT`].
pub const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
