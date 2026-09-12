## MODIFIED Requirements

### Requirement: Usable product reference

The site SHALL document working installation, authentication, terminal controls,
framework concepts, memory, dreaming, configuration precedence, workspace trust,
tools, MCP, and A2A with examples matching the current implementation. It MUST
explain exact-root full-manifest approval invalidation, command-specific
noninteractive behavior, redacted review and diagnostic output, the
non-persistent subset-only one-invocation grant, `config`'s saved-preferences
omission, and that workspace trust does not provide process sandboxing or an
atomic Unix cwd binding.

#### Scenario: First use
- **WHEN** a visitor follows the first conversation guide
- **THEN** they can run the offline demo without credentials and find model discovery and authenticated setup instructions.

#### Scenario: Ancestor authority
- **WHEN** a visitor encounters a workspace-trust prompt or noninteractive refusal
- **THEN** the reference explains the approval and revocation flow without
  suggesting that approval confines shell process authority.
