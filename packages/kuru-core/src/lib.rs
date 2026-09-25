//! Shared contracts for a pool of equal, persistent computational parts.
//!
//! The framework profiles are metaphors for complementary working tendencies.
//! They do not describe clinical treatment or establish machine consciousness.

mod accounting;
mod config;
mod context;
mod framework;
mod model_catalog;
mod permissions;
mod policy;
mod prompt_sources;
mod types;

pub use accounting::{
    InvocationOutcome, InvocationStart, InvocationUsage, MoneyEstimate, SessionUsage,
    UnappliedPriceTerm, UsageCompleteness, UsageObservation, UsagePhase,
};
pub use config::{
    AuthorityClaim, AuthorityClaimCategory, AuthorityManifest, ClaimDigest, Config, ConfigSnapshot,
    HookCommand, HookEvent, InvocationOverrides, LifecycleHooks, ManifestDigest, McpConfig,
    McpOAuthConfig, MemoryConfig, ModelPreference, ProjectPreferences, ResponsesRouteConfig,
    SafeClaimDisplay, SafeManifest, SafeSource, SelectionOverrides, load_instructions,
};
pub use context::{
    ContextBudget, ContextCompactionPolicy, ContextEstimate, ContextSizing, ContextSourceKind,
    ContextSourceSize, ContextTooLarge, DEFAULT_COMPACTION_OUTPUT_RESERVE_TOKENS,
    DEFAULT_COMPACTION_THRESHOLD_PERCENT, DEFAULT_OUTPUT_RESERVE_TOKENS,
    MAX_CONFIGURED_OUTPUT_RESERVE_TOKENS, estimated_tokens_for_bytes,
};
pub use framework::{
    Framework, Mode, Part, Relationship, RelationshipKind, canonical_peer_instruction,
};
pub use model_catalog::{
    ModelCatalog, ModelRoute, TokenizerEncoding, advertised_metadata, enrich_model,
    tokenizer_for_model,
};
pub use permissions::{
    NativeTool, PermissionAction, PermissionRule, PermissionSelector, ProjectRelativeTarget,
};
pub use policy::{
    ActorPhase, ConsolidationPlan, ContextSource, Contribution, FacingDecision, FacingInput,
    FacingPolicy, FlowPolicy, MemoryPolicy, ModeProfile, PeeringPolicy, RelationshipOrigin,
    RolesPolicy, StateKeys, VisibilityPolicy, validate_consolidation_plan,
    validate_context_sources, validate_contributions, validate_facing, validate_identity_namespace,
    validate_peer_edge, validate_recipients, validate_relationship_members,
};
pub use prompt_sources::{CustomCommand, PromptCatalog, SkillMetadata};
pub use types::{
    CacheWriteTerms, Completion, CompletionRequest, ContentBlock, FactProvenance, LongContextTier,
    Message, ModelInfo, ModelMetadata, PriceBasis, PriceSchedule, SourceCitation, Sourced,
    ToolCall, ToolSpec, Usage,
};
