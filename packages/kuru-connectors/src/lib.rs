//! Provider-neutral inference and explicit tool execution. No adapter owns actors
//! or persistent memory; the runtime supplies each actor's permitted context.

mod a2a;
mod auth;
mod http;
mod mcp;
#[cfg(windows)]
mod process;
mod providers;
mod retry;
mod rpc;
mod tools;
#[cfg(unix)]
mod unix_shell;

#[cfg(test)]
mod test_support;

pub use a2a::a2a_send;
pub use auth::{AuthManager, AuthStatus, BrowserLogin, DeviceLogin};
pub use providers::{DemoProvider, Provider, ResponsesProvider, provider};
pub use tools::ToolHost;

/// Maximum protocol message/body size; limits also apply to chunked responses.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
