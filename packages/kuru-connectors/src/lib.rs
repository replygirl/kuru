//! Provider-neutral inference and explicit tool execution. No adapter owns actors
//! or persistent memory; the runtime supplies each actor's permitted context.

mod a2a;
mod codex;
mod http;
mod mcp;
mod providers;
mod rpc;
mod tools;

#[cfg(test)]
mod test_support;

pub use a2a::a2a_send;
pub use codex::{CodexProvider, auth};
pub use providers::{DemoProvider, Provider, ResponsesProvider, provider};
pub use tools::ToolHost;

/// Maximum protocol message/body size; limits also apply to chunked responses.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
