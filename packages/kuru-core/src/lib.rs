//! Shared contracts for a pool of equal, persistent computational parts.
//!
//! The framework profiles are metaphors for complementary working tendencies.
//! They do not describe clinical treatment or establish machine consciousness.

mod config;
mod framework;
mod model_catalog;
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
pub use model_catalog::{ModelCatalog, ModelRoute, advertised_metadata, enrich_model};
pub use types::{
    CacheWriteTerms, Completion, CompletionRequest, ContentBlock, FactProvenance, LongContextTier,
    Message, ModelInfo, ModelMetadata, PriceBasis, PriceSchedule, SourceCitation, Sourced,
    ToolCall, ToolSpec, Usage,
};
