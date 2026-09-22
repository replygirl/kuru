//! Provider-neutral inference and explicit tool execution. No adapter owns actors
//! or persistent memory; the runtime supplies each actor's permitted context.

mod a2a;
mod auth;
mod file_edits;
mod http;
mod instruction_review;
mod mcp;
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

#[cfg(test)]
mod test_support;

pub use a2a::a2a_send;
pub use auth::{AuthManager, AuthStatus, BrowserLogin, DeviceLogin};
pub use file_edits::{CheckpointState, CheckpointStore, CheckpointSummary, FileEffect};
pub use instruction_review::{
    InstructionActivation, InstructionGate, InstructionGateOutcome, InstructionReviewAnswer,
    InstructionReviewRequest, InstructionReviewSender, SkillGate,
};
pub use mcp::McpStatus;
pub use permissions::{
    ApprovalAnswer, ApprovalRequest, ApprovalSender, GrantInspection, GrantRevocation, GrantScope,
    PermissionBinding, PermissionDisplay, PermissionGrantStore, PermissionInvocation,
    PermissionOutcome, PermissionService, PersistentGrant,
};
pub use providers::{
    DemoProvider, Provider, ProviderEvent, ProviderFailureKind, ProviderSink, ResponsesProvider,
    TextDeltaSource, collect_completion, provider,
};
pub use redaction::{
    ProjectionError, json as project_json, text as project_text, truncate_tool_output,
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
