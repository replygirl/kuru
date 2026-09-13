## MODIFIED Requirements

### Requirement: Usable product reference

The site SHALL document working installation, authentication, terminal controls, framework concepts, memory, dreaming, configuration precedence, workspace trust, tools, MCP, A2A, and bounded/redacted provider-failure behavior with examples matching the current implementation. It MUST explain exact-root full-manifest approval invalidation, command-specific noninteractive behavior, redacted review and diagnostic output, the non-persistent subset-only one-invocation grant, `config`'s saved-preferences omission, and that workspace trust does not provide process sandboxing or an atomic Unix cwd binding. It MUST state that provider failure text is classified from bounded input rather than treated as authoritative remote text. It MUST explain that memory startup messages describe current verified work on stderr, do not estimate completion or reduce integrity checks, and that an unsafe legacy Unix data directory requires an owner-only permission repair before import.

#### Scenario: First use
- **WHEN** a visitor follows the first conversation guide
- **THEN** they can run the offline demo without credentials and find model discovery and authenticated setup instructions

#### Scenario: Ancestor authority
- **WHEN** a visitor encounters a workspace-trust prompt or noninteractive refusal
- **THEN** the reference explains the approval and revocation flow without suggesting that approval confines shell process authority.

#### Scenario: Provider failure contract
- **WHEN** a visitor reads the provider protocol reference
- **THEN** it explains that completion and model-catalog failures use fixed, bounded, redacted diagnostics without exposing provider body text

#### Scenario: Memory startup and legacy import
- **WHEN** a visitor encounters verified memory startup work or a legacy privacy refusal
- **THEN** the documentation distinguishes fixed progress from completion and gives the applicable owner-private recovery guidance without promising an automatic repair.
