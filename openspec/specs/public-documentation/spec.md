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
framework concepts, memory, dreaming, configuration precedence, workspace trust,
tools, MCP, A2A, and bounded/redacted provider-failure behavior with examples
matching the current implementation. It MUST explain exact-root full-manifest
approval invalidation, command-specific noninteractive behavior, redacted review
and diagnostic output, the non-persistent subset-only one-invocation grant,
`config`'s saved-preferences omission, and that workspace trust does not provide
process sandboxing or an atomic Unix cwd binding. It MUST state that provider
failure text is classified from bounded input rather than treated as authoritative
remote text. The memory reference MUST document selected-note sequence IDs and
explicit forgetting as an active-view deletion that retains prior Dolt revisions
and does not claim secure erasure, removal of other text, automatic expiry, or a
restore command.

It MUST describe that catalog and completion retries are bounded by one
operation deadline and finite attempts, apply only to selected rejected provider
responses before a response is accepted, and do not promise replay after a
stream, partial output, ambiguous transport outcome, or possibly dispatched
credential refresh.

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
