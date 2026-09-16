//! Shared contracts for a pool of equal, persistent computational parts.
//!
//! The framework profiles are metaphors for complementary working tendencies.
//! They do not describe clinical treatment or establish machine consciousness.

mod config;
mod framework;
mod types;

pub use config::{
    AuthorityClaim, AuthorityClaimCategory, AuthorityManifest, ClaimDigest, Config, ConfigSnapshot,
    InvocationOverrides, ManifestDigest, McpConfig, MemoryConfig, ModelPreference,
    ProjectPreferences, ResponsesRouteConfig, SafeClaimDisplay, SafeManifest, SafeSource,
    SelectionOverrides, load_instructions,
};
pub use framework::{
    Framework, Mode, Part, Relationship, RelationshipKind, canonical_peer_instruction,
};
pub use types::{
    Completion, CompletionRequest, ContentBlock, Message, ModelInfo, ToolCall, ToolSpec, Usage,
};
