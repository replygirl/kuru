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

The site SHALL document working installation, authentication, terminal controls, framework concepts, memory, dreaming, configuration precedence, workspace trust, tools, MCP, A2A, and bounded/redacted provider-failure behavior with examples matching the current implementation. It MUST explain exact-root full-manifest approval invalidation, command-specific noninteractive behavior, redacted review and diagnostic output, the non-persistent subset-only one-invocation grant, `config`'s saved-preferences omission, and that workspace trust does not provide process sandboxing or an atomic Unix cwd binding. It MUST state that provider failure text is classified from bounded input rather than treated as authoritative remote text. The memory reference MUST document selected-note sequence IDs and explicit forgetting as an active-view deletion that retains prior Dolt revisions and does not claim secure erasure, removal of other text, automatic expiry, or a restore command. It MUST explain the one-time informational memory notice, its actual-directory/control content, and that explicit project purge remains a separate confirmed managed-history operation with detailed retained-copy help.

It MUST describe that catalog and completion retries are bounded by one operation deadline and finite attempts, apply only to selected rejected provider responses before a response is accepted, and do not promise replay after a stream, partial output, ambiguous transport outcome, or possibly dispatched credential refresh. It MUST document that `kuru memory export` produces one committed active-main snapshot of every application message and state record, identifies its revision and schema, preserves stored content including unknown fields, and does not erase or rewrite history. It MUST distinguish JSON as the interchange format from Markdown's equivalent rendering, explain stdout versus checked no-replacement file output, and state that uncommitted, candidate-only, prior-revision, and operational data are excluded. It MUST explain that memory startup messages describe current verified work on stderr, do not estimate completion or reduce integrity checks, and that an unsafe legacy Unix data directory requires an owner-only permission repair before import.

#### Scenario: Selected-note retention disclosure
- **WHEN** a visitor reads the memory command reference
- **THEN** it explains how to select one stored note and that forgetting leaves prior revisions and unrelated text intact.

#### Scenario: Memory snapshot export
- **WHEN** a visitor reads the memory command reference
- **THEN** they can request a committed full-project JSON or Markdown snapshot
  without inferring that it deletes, redacts, or includes candidate working data

#### Scenario: Memory startup and legacy import
- **WHEN** a visitor encounters verified memory startup work or a legacy privacy refusal
- **THEN** the documentation distinguishes fixed progress from completion and gives the applicable owner-private recovery guidance without promising an automatic repair.

#### Scenario: First use
- **WHEN** a visitor follows the first conversation guide
- **THEN** they can run the offline demo without credentials and find model discovery and authenticated setup instructions.

#### Scenario: First-run memory controls
- **WHEN** a visitor reads the memory concept reference
- **THEN** it distinguishes the informational first-run notice from consent and identifies notes, export, selected forgetting, and confirmed purge boundaries.

#### Scenario: Ancestor authority
- **WHEN** a visitor encounters a workspace-trust prompt or noninteractive refusal
- **THEN** the reference explains the approval and revocation flow without suggesting that approval confines shell process authority.

#### Scenario: Provider failure contract
- **WHEN** a visitor reads the provider protocol reference
- **THEN** it explains that completion and model-catalog failures use fixed, bounded, redacted diagnostics without exposing provider body text.

#### Scenario: Provider retry contract
- **WHEN** a visitor reads the provider protocol reference
- **THEN** it explains finite retry and Retry-After limits without claiming that Kuru replays accepted streams, ambiguous requests, or rotating credential POSTs.

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
guarantee that arbitrary secrets are private. The inventory MUST include the
documented Slack and GitLab prefixes, JWT header-shape requirement, URL-userinfo
form, and bare `token`/`secret` assignment forms.

#### Scenario: User interprets a projected result

- **WHEN** a user sees `[REDACTED:recognized-secret]` in file, shell or MCP
  output
- **THEN** the tools reference explains which returned projection changed, which
  original data was preserved and why unknown or transformed secrets may remain
  visible

#### Scenario: User interprets an expanded projected result

- **WHEN** a user sees `[REDACTED:recognized-secret]` for an added credential
  form in file, shell or MCP output
- **THEN** the tools reference explains the returned projection, recognized
  finite form, preserved original data and why unknown or transformed secrets
  may remain visible

### Requirement: Published release verification documentation

Maintainer documentation SHALL describe the same-run automated staged Windows
gate, its relationship to documentation deployment and final public release
promotion, and recovery by rerunning the existing Release workflow. It SHALL
state that the loopback fixture consumes the exact staged Windows ZIP without
claiming an actual public download, and distinguish the separately invokable
published-package verifier. Curated installation documentation SHALL describe
the compiler-free binary verification path without exposing repository-only
evidence.

#### Scenario: Maintainer recovers post-publication verification
- **WHEN** staged Windows acceptance, documentation build, or documentation deployment fails or is interrupted
- **THEN** the release guide directs the maintainer to rerun the failed work in the same Release run and explains that no public release is promoted until every dependency of the final publication job succeeds

#### Scenario: Visitor reads installation guidance
- **WHEN** a visitor selects the mise installation path
- **THEN** the guide accurately distinguishes pre-publication native acceptance of the exact staged ZIP from an actual public download while preserving the compiler-free installation instructions

### Requirement: MCP degradation reference

The curated documentation SHALL explain that MCP tools are discovered per configured server, failed servers are reported without hiding built-in or healthy-server tools, failed or ambiguously cancelled calls are not replayed, and recovery occurs only through a later explicit discovery. It MUST explain that bounded stdio stderr is recognizable-secret projected and terminal escaped for human diagnostics only, while runtime status is fixed metadata, and MUST retain the documented finite-detector, process-authority, and no-sandbox limitations.

#### Scenario: User diagnoses one failed server

- **WHEN** a configured MCP server fails while another server remains healthy
- **THEN** the tools and protocol references explain which tools remain available, where the safe status and optional stderr diagnostic appear, and why Kuru neither replays the failed call nor claims arbitrary-secret or escaped-process containment

### Requirement: Honest memory maintenance documentation

The owning memory documentation SHALL explain automatic Dolt GC, mutation
receipt replacement and resolved candidate reclamation. It MUST state that Kuru
does not automatically expire conversations or notes, preserves reachable Dolt
revisions and unresolved candidates, continues to grow with retained history,
and does not provide secure erasure through GC.

#### Scenario: User reviews retention behavior

- **WHEN** a user reads either memory guide to understand storage growth or deletion
- **THEN** they can distinguish operational cleanup from user-history retention and find no automatic-expiry or secure-erasure claim.

### Requirement: Explicit project-purge boundary

Curated documentation SHALL explain the confirmed project-memory purge command,
that it removes the selected managed Dolt current store and revision history, and
that it does not promise secure physical erasure. It MUST distinguish retained
original/shared legacy SQLite, user exports/backups, engine cache, and other
projects, and state that selected-note forgetting retains historical revisions.

#### Scenario: User evaluates deletion scope
- **WHEN** a user reads memory controls before confirming a purge
- **THEN** they can distinguish managed project-history removal from retained
  shared/original copies and do not receive a secure-erasure or automatic-expiry
  promise.

### Requirement: Native search and paging reference

Curated documentation SHALL identify `grep`, `glob`, and `file_read` paging
as checked project-root tools, document their permission behavior, bounds,
binary handling, hidden and ignore defaults, and state that native search does
not need an installed `rg` executable. It SHALL define paging as one-based
logical UTF-8 text lines and explain returned continuation and omission fields.

#### Scenario: User pages a large text file
- **WHEN** a user reads the built-in-tools reference before using file_read
- **THEN** they can request a line offset and limit and use next_offset without
  inferring byte, Unicode-scalar, or external-tool semantics.

### Requirement: MCP catalog-control reference

Public configuration and tools documentation SHALL describe server enabled state, original-name allow and deny precedence, stale-cache status, explicit live recovery, environment-referenced static headers, and the distinction between catalog metadata, availability, workspace trust, and execution permission. It MUST state that header values stay in their configured environment variables and that OAuth, MCP resources, prompts, and sampling are outside this capability.

#### Scenario: User configures a controlled MCP catalog
- **WHEN** a user reads the MCP configuration and tools reference
- **THEN** they can configure a disabled or filtered server, interpret live/stale/degraded status, supply a static header by environment reference, and understand that a listed tool still requires a current route and permission

### Requirement: MCP OAuth and native credential reference

Curated configuration, tools and protocol documentation SHALL explain the MCP 2026-07-28 HTTP authorization contract as applied to Kuru's existing negotiated transport: OAuth discovery, configured/CIMD/deprecated-compatibility-DCR registration order, DCR native application type, PKCE, RFC 9207 issuer validation, authoritative initial and step-up scope selection, resource binding, browser and advertised device login, same-host or forwarded loopback requirements for no-browser use, refresh behavior, native credential-store prerequisites, per-alias status and logout. It MUST avoid claiming full MCP 2026 wire-protocol support; state that Kuru stores only its own MCP credentials, has no plaintext or session-only fallback, keeps OpenAI and static-authorization routes separate, limits non-authorization static headers to protected resource requests, does not replay ambiguous tool or rotating-refresh requests, and deletes local credentials while reporting advertised remote revocation separately and without promising it occurred when absent or failed.

#### Scenario: User prepares a headless MCP login
- **WHEN** a user reads the MCP authorization guide from a host without a usable local browser
- **THEN** they can determine whether loopback forwarding is required, how to select an advertised device flow, what native credential facility is required and how cancellation affects stored state

#### Scenario: User interprets logout
- **WHEN** logout reports local deletion together with no advertised revocation endpoint or a remote refusal
- **THEN** the documentation explains that Kuru's credential is gone without claiming the remote authorization grant was revoked

### Requirement: Accurate shell-support installation guidance

Installation documentation SHALL name the supported completion shells, stable Unix `share/man/man1/kuru.1` location, direct-install versioned support directory, and stock-shell activation commands using the fixed installed executable's pure generator rather than a profile path pinned to one versioned support directory, without modifying a user's profile automatically. It SHALL show one-time MANPATH setup for ordinary `man kuru`, distinguish shipped assets from activated completions, say that a running shell needs a reload after a settled update, and avoid promising synchronized activation during an unresolved replacement. Older already-published updaters can upgrade the compatible core binary but cannot retroactively install a new support envelope; rerunning the current verified installer or using the new binary's pure output commands SHALL provide a concrete repair path. Homebrew tap/formula installation SHALL be documented only when the separate P24b integration is actually delivered.

#### Scenario: Direct installer completed
- **WHEN** a user has installed Kuru into an explicit directory
- **THEN** the docs show how each stock shell and Unix man use pure output from the fixed installed executable, and separately where the exact versioned files live, without assuming an edited profile or leaving a persistent path pinned to an obsolete version.

#### Scenario: Legacy updater completed
- **WHEN** an older client upgraded only the unchanged executable archive
- **THEN** the docs show how to rerun the current installer to obtain managed support or generate and place matching user-managed shell/man bytes with the newly installed binary, without claiming that the older updater installed them.
