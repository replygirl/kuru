//! Shared contracts for a pool of equal, persistent computational parts.
//!
//! The framework profiles are metaphors for complementary working tendencies.
//! They do not describe clinical treatment or establish machine consciousness.

mod config;
mod framework;
mod memory;
mod types;

pub use config::{Config, McpConfig, load_instructions};
pub use framework::{Framework, Mode, Part, Relationship, RelationshipKind};
pub use memory::MemoryStore;
pub use types::{Completion, CompletionRequest, Message, ModelInfo, ToolCall, ToolSpec};
