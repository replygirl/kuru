## MODIFIED Requirements

### Requirement: Usable product reference

The site SHALL document working installation, authentication, terminal controls,
framework concepts, memory, dreaming, configuration precedence, workspace trust, tools, MCP, A2A,
and bounded/redacted provider-failure behavior with examples matching the current
implementation. It MUST explain exact-root full-manifest approval invalidation,
command-specific noninteractive behavior, redacted review and diagnostic output,
the non-persistent subset-only one-invocation grant, `config`'s saved-preferences
omission, and that workspace trust does not provide process sandboxing or an
atomic Unix cwd binding. It MUST state that provider failure text is classified from
bounded input rather than treated as authoritative remote text. It MUST describe
that catalog and completion retries are bounded by one operation deadline and
finite attempts, apply only to selected rejected provider responses before a
response is accepted, and do not promise replay after a stream, partial output,
ambiguous transport outcome, or possibly dispatched credential refresh. It MUST
document that `kuru memory export` produces one committed active-main snapshot of
every application message and state record, identifies its revision and schema,
preserves stored content including unknown fields, and does not erase or rewrite
history. It MUST distinguish JSON as the interchange format from Markdown's
equivalent rendering, explain stdout versus checked no-replacement file output,
and state that uncommitted, candidate-only, prior-revision, and operational data
are excluded.

#### Scenario: First use
- **WHEN** a visitor follows the first conversation guide
- **THEN** they can run the offline demo without credentials and find model
  discovery and authenticated setup instructions

#### Scenario: Ancestor authority
- **WHEN** a visitor encounters a workspace-trust prompt or noninteractive refusal
- **THEN** the reference explains the approval and revocation flow without
  suggesting that approval confines shell process authority.

#### Scenario: Provider failure contract
- **WHEN** a visitor reads the provider protocol reference
- **THEN** it explains that completion and model-catalog failures use fixed,
  bounded, redacted diagnostics without exposing provider body text

#### Scenario: Provider retry contract
- **WHEN** a visitor reads the provider protocol reference
- **THEN** it explains finite retry and Retry-After limits without claiming that
  Kuru replays accepted streams, ambiguous requests, or rotating credential POSTs

#### Scenario: Memory snapshot export
- **WHEN** a visitor reads the memory command reference
- **THEN** they can request a committed full-project JSON or Markdown snapshot
  without inferring that it deletes, redacts, or includes candidate working data
