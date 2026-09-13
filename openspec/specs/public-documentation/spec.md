# public-documentation Specification

## Purpose
Provide accessible, curated Kuru documentation with accurate setup and reference
guides, searchable static output, and interactive previews of the framework
models without publishing repository-only records or private state.

## Requirements

### Requirement: Curated public content

The documentation site MUST publish only deliberately authored product documentation from `apps/kuru-docs`. It MUST NOT automatically publish repository engineering notes, local paths, credentials, or account-specific verification records.

#### Scenario: Production build
- **WHEN** the documentation production build runs
- **THEN** HTML, search data, and machine-readable documentation contain the curated user pages and no repository-only records

### Requirement: Usable product reference

The site SHALL document working installation, authentication, terminal controls,
framework concepts, memory, dreaming, configuration precedence, workspace trust, tools, MCP, A2A,
and bounded/redacted provider-failure behavior with examples matching the current
implementation. It MUST explain exact-root full-manifest approval invalidation,
command-specific noninteractive behavior, redacted review and diagnostic output,
the non-persistent subset-only one-invocation grant, `config`'s saved-preferences
omission, and that workspace trust does not provide process sandboxing or an
atomic Unix cwd binding. It MUST state that provider failure text is classified from
bounded input rather than treated as authoritative remote text. The memory
reference MUST document selected-note sequence IDs and explicit forgetting as an
active-view deletion that retains prior Dolt revisions and does not claim secure
erasure, removal of other text, automatic expiry, or a restore command. It MUST
describe that catalog and completion retries are bounded by one operation deadline and
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

#### Scenario: Selected-note retention disclosure
- **WHEN** a visitor reads the memory command reference
- **THEN** it explains how to select one stored note and that forgetting leaves prior revisions and unrelated text intact

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

### Requirement: Accessible navigation and product identity

The site SHALL provide dark and light themes, responsive navigation, local search, visible keyboard focus, readable contrast, and framework graphics with equivalent text. Decorative motion MUST respect reduced-motion preferences and MUST NOT move article content.

#### Scenario: Narrow viewport and keyboard input
- **WHEN** a visitor uses a narrow viewport or navigates with a keyboard
- **THEN** navigation, framework selection, content, and search remain available without horizontal page overflow

### Requirement: Project Pages compatibility

The application SHALL build a static site for the `/kuru/` project path with correct internal links, search assets, and direct page navigation.

#### Scenario: Production preview
- **WHEN** the built site is served at `/kuru/`
- **THEN** landing pages, nested reference pages, and local search load successfully

### Requirement: Tool-result projection reference

The curated documentation SHALL describe recognizable-secret projection for
successful and failed tool results, show the exact visible marker, enumerate the
finite detector categories and local heuristic floors, and state the known
false-positive and false-negative limits. It MUST explain that the projection
does not inspect credential stores or enumerate environment values, does not
modify tool inputs, writes or prior history, and is not a byte-exact backup or a
guarantee that arbitrary secrets are private.

#### Scenario: User interprets a projected result

- **WHEN** a user sees `[REDACTED:recognized-secret]` in file, shell or MCP output
- **THEN** the tools reference explains which returned projection changed, which original data was preserved and why unknown or transformed secrets may remain visible

### Requirement: Published release verification documentation

Maintainer documentation SHALL describe the same-run automated published
Windows gate, its bounded receipt, its relationship to final documentation
publication, and recovery by rerunning the existing Release workflow job. Curated
installation documentation SHALL describe published binary verification without
exposing repository-only evidence or suggesting that ordinary installation
requires a compiler.

#### Scenario: Maintainer recovers post-publication verification
- **WHEN** publication succeeded but the published Windows verification job failed or was interrupted
- **THEN** the release guide directs the maintainer to rerun that job in the same Release run and explains that documentation remains blocked until it succeeds

#### Scenario: Visitor reads installation guidance
- **WHEN** a visitor selects the mise installation path
- **THEN** the guide accurately states that the published Windows package is checked through the native same-run release gate while preserving the compiler-free installation instructions

### Requirement: MCP degradation reference

The curated documentation SHALL explain that MCP tools are discovered per configured server, failed servers are reported without hiding built-in or healthy-server tools, failed or ambiguously cancelled calls are not replayed, and recovery occurs only through a later explicit discovery. It MUST explain that bounded stdio stderr is recognizable-secret projected and terminal escaped for human diagnostics only, while runtime status is fixed metadata, and MUST retain the documented finite-detector, process-authority, and no-sandbox limitations.

#### Scenario: User diagnoses one failed server

- **WHEN** a configured MCP server fails while another server remains healthy
- **THEN** the tools and protocol references explain which tools remain available, where the safe status and optional stderr diagnostic appear, and why Kuru neither replays the failed call nor claims arbitrary-secret or escaped-process containment
