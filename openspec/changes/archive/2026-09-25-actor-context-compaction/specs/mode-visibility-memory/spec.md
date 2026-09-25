## ADDED Requirements

### Requirement: Effective policies govern compaction source and summary continuity

Before reading a compaction source or a settled compaction summary, the runtime SHALL obtain and validate the selected mode's effective visibility and memory-policy decisions for that exact actor phase and namespace. Automatic and manual compaction MUST summarize only policy-admitted raw current-session history. Later ordinary requests MAY include current-session or cross-session cursor-selected context summaries only when policy admits that summary namespace. Policy admission MUST NOT convert producer-private reasoning summaries, including operation-attributed Compact sidecars, public transcript rows, notes or another actor's raw history into compaction source.

#### Scenario: Policy omits history or cross-session summaries
- **WHEN** a test policy omits an actor's raw history or cross-session summary continuity
- **THEN** compaction refuses or excludes that source before reading it, and later context contains neither omitted summaries nor a fallback namespace.

#### Scenario: Policy changes after snapshot
- **WHEN** effective policy or actor namespace changes after a source snapshot but before publication
- **THEN** revalidation prevents publication under stale authority and leaves the previous summary/cursor projection usable.
