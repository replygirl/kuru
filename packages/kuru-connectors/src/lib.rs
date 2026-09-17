//! Provider-neutral inference and explicit tool execution. No adapter owns actors
//! or persistent memory; the runtime supplies each actor's permitted context.

mod a2a;
mod auth;
mod http;
mod mcp;
#[cfg(windows)]
mod process;
mod providers;
mod redaction;
mod retry;
mod rpc;
mod shell_diagnostic;
mod tool_output;
mod tools;
#[cfg(unix)]
mod unix_shell;

#[cfg(test)]
mod test_support;

pub use a2a::a2a_send;
pub use auth::{AuthManager, AuthStatus, BrowserLogin, DeviceLogin};
pub use mcp::McpStatus;
pub use providers::{
    DemoProvider, Provider, ProviderEvent, ProviderFailureKind, ProviderSink, ResponsesProvider,
    TextDeltaSource, collect_completion, provider,
};
pub use redaction::{
    ProjectionError, json as project_json, text as project_text, truncate_tool_output,
};
pub use tool_output::is_permission_denied;
pub use tools::{ToolCatalog, ToolHost};

/// Maximum protocol message/body size; limits also apply to chunked responses.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
