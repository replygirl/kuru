# workspace-trust Specification

## Purpose
Require explicit approval before automatically discovered workspace configuration
activates process, filesystem, memory-engine or provider authority. Preserve a
reviewable immutable configuration, exact-root approval identity, private grant
storage and side-effect-free inspection without claiming process sandboxing.

## Requirements

### Requirement: Immutable provenance-aware workspace configuration

Kuru SHALL parse user, automatic ancestor, explicit configuration and caller
overrides once into an immutable, side-effect-free snapshot rooted at the exact
canonical `-C` directory. The snapshot MUST retain field-level final-writer
provenance for security-relevant scalar and map leaves, capture every CLI
override including `allow_write`, `allow_shell`, and `no_dream`, and capture the
ordered identity and exact bytes of every automatically discovered instruction
source, including the project-root `AGENTS.md`. It MUST derive a canonical,
versioned authority manifest only from effective automatic-ancestor
configuration and automatic instruction sources. Parsing and manifest
derivation MUST NOT open memory, read credentials or configured environment
variables, start a process, connect to a configured endpoint, or inject
instructions into a provider prompt. No post-snapshot mutation may add or alter
authority configuration or instructions; ordinary model auto-resolution and
runtime preference application MAY occur after the final snapshot without
rereading configuration or instruction files. Saved preferences MAY affect only
mode, model and effort after preflight; they MUST NOT expand the manifest.

#### Scenario: Overridden ancestor authority leaf
- **WHEN** an explicit caller value overrides one authority-bearing leaf while an
  automatic ancestor still contributes another effective leaf in the same map
- **THEN** the manifest includes only the remaining ancestor-contributed leaf.

#### Scenario: Ordinary ancestor defaults
- **WHEN** automatic ancestors contribute only mode, model, effort, budgets,
  dreaming settings, memory offline state or timeouts and no automatic
  instruction source exists
- **THEN** no workspace-trust approval is required and ordinary startup remains
  available.

#### Scenario: Ordered automatic instructions
- **WHEN** automatic instruction sources exist at multiple ancestors including
  the canonical project root
- **THEN** one typed instruction claim binds every source identity and exact
  content in outermost-to-most-local order without a home-directory exemption.

#### Scenario: Malformed automatic configuration
- **WHEN** an automatic or explicit configuration file has a read, TOML parse,
  type, or validation error containing fake secrets or terminal controls
- **THEN** Kuru reports only a bounded escaped source label, coarse error
  category, and parser-provided line and column when available, without raw
  parser text, excerpts, values, or control characters.

### Requirement: Exact-root private approval state

Kuru SHALL bind persistent workspace approval to the normalized canonical root
path, current native directory identity, complete manifest schema and digest,
and sorted claim digests. The complete manifest digest MUST also bind every
contributing automatic configuration or instruction source's full path identity
through an unambiguous private encoding or digest, never its truncated display
label. Authority-neutral configuration edits at the same source path MUST NOT
invalidate approval. Adding, removing, reordering, changing or replacing an
automatic instruction source MUST invalidate approval. Only an explicit `trust
approve` action or the full-manifest persistent TUI choice MAY create or update
persistent approval; the stored claim digests are audit data and MUST NOT act as
partial grants. A matching complete approval MAY satisfy a command's applicable
claim subset, but any authority addition, removal or value change MUST
invalidate the entire stored approval. Approval state MUST be checked, private,
bounded, strict-schema, atomically updated outside tool roots and Dolt, and MUST
NOT retain raw configuration values, instruction contents or redacted display
strings. Approvals MUST NOT inherit to parent or child roots. Invalid, unsafe,
replaced, oversized, linked or uncertain approval state MUST fail closed before
authority activation.

#### Scenario: Changed authority or root identity
- **WHEN** an authority claim changes, the normalized root changes, or the root
  object at the same path is replaced
- **THEN** the stored approval does not match and activation requires a new
  explicit approval.

#### Scenario: Changed instruction source
- **WHEN** an automatic instruction source is added, removed, reordered,
  replaced or changed after approval
- **THEN** the complete stored approval is stale even if all configuration
  authority is unchanged.

#### Scenario: Authority-neutral configuration change
- **WHEN** only an ordinary setting changes while the exact root identity and
  canonical authority manifest remain unchanged
- **THEN** the matching approval remains valid.

#### Scenario: Subset command after approval
- **WHEN** a complete current-manifest approval matches and a command needs only
  a subset of its claims
- **THEN** that command may use its subset without creating a narrower record
  or treating individual claim digests as grants.

### Requirement: Command-specific workspace preflight

Before activation, Kuru SHALL resolve the applicable subset of the workspace
authority manifest against a matching complete approval or an explicit
one-invocation grant; a noninteractive command MUST fail promptly rather than
wait for input. `--trust-workspace-once` SHALL authorize only the invoking
command's applicable subset and MUST NOT write, broaden or persist approval.
`trust status` SHALL inspect any existing checked record and an absent-record
`trust revoke` SHALL be a no-op; neither may create trust state, directory, lock
or record. Only explicit persistent approval may create state. `config` SHALL
inspect without activation, MemoryStore opening, memory migration, provisioning
or a writer lease and SHALL truthfully omit saved project preferences when no
safe already-live attach-only API exists. `login` and `logout` SHALL use only
their fixed ChatGPT route, `auth` SHALL gate any Responses environment check,
memory and `undo-dream` commands SHALL gate configured memory paths without
constructing a provider or reading provider credentials, and full runtime
commands SHALL gate every applicable instruction, memory, provider, write,
shell, MCP and external-agent claim. Catalog and direct-tool commands SHALL gate
their applicable configuration claims without consuming instruction authority
when they do not construct prompts.

#### Scenario: Unapproved automatic instruction
- **WHEN** an automatic instruction source exists and a user invokes `run`
  without matching approval or a one-invocation grant
- **THEN** Kuru constructs no provider or runtime and injects no instruction
  bytes.

#### Scenario: Non-prompt inspection
- **WHEN** automatic instruction sources exist and the user invokes fixed
  authentication, configuration, trust, memory, catalog or direct-tool
  inspection that does not construct a runtime
- **THEN** instruction authority is not part of that command's applicable
  subset and no instruction reaches a provider.

#### Scenario: Unapproved configured transport
- **WHEN** an automatic ancestor contributes a stdio or HTTP MCP claim and a
  user invokes `tools` without matching approval or a one-invocation grant
- **THEN** no child process is spawned, no socket is opened, and the command
  reports a bounded redacted manifest with an explicit remedy.

#### Scenario: Unapproved Responses route
- **WHEN** an automatic ancestor contributes an active Responses provider tuple
  and approval is absent
- **THEN** Kuru does not read the selected environment variable or send a
  request to its configured API base.

#### Scenario: Inspection without a memory store
- **WHEN** the user invokes `config` against an unapproved workspace or an
  absent data directory
- **THEN** Kuru prints parseable snapshot TOML and a bounded saved-preferences
  omission notice without creating a memory, provisioning Dolt, or acquiring a
  writer lease.

#### Scenario: Undo without provider activation
- **WHEN** the user invokes `undo-dream` with an ancestor Responses route and
  only the applicable configured-memory claim is approved
- **THEN** Kuru performs no provider construction, API-key environment read, or
  provider request.

### Requirement: Redacted reviewable trust flow

Kuru SHALL provide `trust status`, `trust approve`, `trust revoke`, and
`--trust-workspace-once`. Status and approval output MUST show the normalized
root, contributing automatic configuration and instruction sources and bounded
escaped claims without printing instruction contents, MCP arguments or
environment values, credential values, unescaped controls or URL queries. The
instruction claim MUST preserve and expose source order safely. The interactive
TUI SHALL resolve trust before entering the alternate screen and offer one-time
continuation, full-manifest persistent approval or cancellation. Revocation
SHALL only remove authority and require no confirmation. All configuration and
preflight diagnostics MUST use bounded escaped source labels, coarse
read/parse/type/validation categories, and only parser-provided positions; they
MUST NOT render raw error chains, parser excerpts, values or terminal controls.

#### Scenario: Safe ordered instruction review
- **WHEN** trust status or approval reviews multiple automatic instruction sources
- **THEN** it shows bounded source labels in precedence order and a fixed claim
  description without printing their contents.

#### Scenario: Interactive refusal
- **WHEN** the pre-TUI trust choice is refused or receives EOF
- **THEN** no approval is persisted and no memory, provider, tool, configured
  endpoint or instruction is activated.

#### Scenario: Read-only absent state
- **WHEN** `trust status` or `trust revoke` is run for a workspace with no
  approval record
- **THEN** it reports unapproved or no-op without creating a trust directory,
  lock or record.

### Requirement: Retained-root launch revalidation

Kuru SHALL retain and revalidate the canonical workspace directory after parsing
and before approval or activation. The TUI SHALL pass that retained capability
through `Harness` and `ToolHost` to configured shell and stdio-MCP launches;
those paths MUST use and revalidate the original guard and MUST NOT replace it
with an independently reopened workspace guard after approval. Capability-root
opens remain permitted when bracketed by original-guard revalidation. It SHALL
revalidate that retained directory
immediately before shell and stdio MCP launches that use the workspace pathname
as their child working directory, and fail closed on an observed replacement.
Kuru SHALL NOT claim this check atomically binds a Unix child cwd to the held
directory or provides an OS sandbox.

#### Scenario: Workspace replacement before spawn
- **WHEN** the workspace pathname is replaced after preflight and before a
  configured cwd-based shell or stdio MCP launch
- **THEN** revalidation rejects the launch without starting the configured child.

### Requirement: Reviewed automatic permission authority

Every effective permission rule supplied by automatic ancestor configuration SHALL be included with final-leaf provenance in the immutable exact-root authority manifest before its affected tool or protocol authority activates. Approval of that workspace manifest SHALL authorize use of the reviewed configuration, but SHALL NOT itself create a tool grant or override a matching permission deny. Invocation-only trust SHALL remain scoped to that command's applicable claims, and changing the reviewed manifest SHALL invalidate always grants bound to the former authority context.

#### Scenario: Repo rule cannot activate before review
- **WHEN** an automatic ancestor adds an allow or ask rule affecting a tool invocation without matching workspace trust
- **THEN** Kuru may show a bounded redacted claim but does not construct or execute that configured authority before applicable review.

#### Scenario: Trust is not tool approval
- **WHEN** a workspace manifest is approved but an invocation's effective decision is ask or deny
- **THEN** ask still needs a valid foreground/grant decision and deny still performs no effect.

### Requirement: Distinct project-local and managed provenance

Kuru SHALL discover `.kuru/config.local.toml` only at the exact canonical workspace root. The file SHALL be bounded and checked as a regular file, and when the root is inside a Git worktree Kuru SHALL reject the local file if it is tracked or if its untracked status cannot be established. A valid local file SHALL be explicit user-local authority outside the automatic repository trust manifest. Managed configuration SHALL be loaded only from an explicitly provisioned external absolute path outside the workspace. The immutable snapshot SHALL retain final-leaf provenance across these layers, and a local or CLI value MUST NOT reclassify a remaining repository-origin leaf or instruction as user authority.

#### Scenario: Tracked local file
- **WHEN** a repository tracks `.kuru/config.local.toml`
- **THEN** Kuru rejects the file as a local override before activation and offers a bounded remedy.

#### Scenario: Untracked local and repo authority coexist
- **WHEN** an untracked local file changes one leaf and an ancestor config still contributes an effective MCP or tool-permission leaf
- **THEN** local authority needs no trust approval, while the remaining repository-origin claim still requires applicable workspace review.

#### Scenario: Dead repository authority
- **WHEN** an explicit local value disables a repository-contributed shell default
- **THEN** no shell authority claim is required for that now-ineffective repository value.

### Requirement: Imported instruction authority is captured before review

Kuru SHALL derive the applicable project-instruction claim from the rendered encounter order of all active `AGENTS.md`, `CLAUDE.md` and imported sources, binding each full path digest, checked containing-directory and file identity, and exact captured bytes. An omitted over-cap source SHALL NOT activate prompt authority; a subsequently fitting or changed source SHALL produce a newly reviewed manifest. Kuru SHALL inject only the captured projection after the complete applicable workspace manifest passes the existing exact-root once or persistent approval preflight. It MUST NOT reopen an instruction source after that review to build the same invocation's prompt.

#### Scenario: Imported bytes change after approval
- **WHEN** an imported source changes after approval but before dispatch in the current invocation
- **THEN** the current invocation uses only its captured reviewed bytes, and a new snapshot sees a different manifest requiring fresh review before the changed bytes can activate.

#### Scenario: Explicit local override does not bless import
- **WHEN** a user-owned local config overrides a repository setting and a repository `CLAUDE.md` imports another instruction file
- **THEN** the effective override keeps user provenance while the imported prompt bytes remain a separate repository-origin claim requiring workspace review.

### Requirement: Review newly encountered instruction authority before continuation

Kuru SHALL derive a new complete, exact-root authority manifest from the existing immutable configuration snapshot and the exact checked nested instruction bytes captured for an authorized target. A matching stored approval for that complete manifest or a fresh applicable one-invocation grant SHALL be required before new bytes activate. During an interactive turn, Kuru SHALL offer the existing in-process once, persistent full-manifest, and deny choices before continuing; refusal or a closed review surface SHALL leave the new bytes inactive and prevent the pending target's effect or result exposure. A headless turn without a matching approval or explicit applicable one-invocation grant SHALL return a bounded trust-required result with a concrete review remedy and no unreviewed instruction injection. Tool permission approval and workspace trust SHALL remain separate decisions. A persistent nested approval SHALL preserve root-only startup approval, bind the exact active nested source set and complete extended manifest, and be removed by ordinary `trust revoke` without scanning unrelated project paths at startup.

#### Scenario: Interactive new nested claim
- **WHEN** an authorized actor read first reaches a nested source absent from the currently approved manifest
- **THEN** the foreground review shows the changed complete manifest without source contents and no target result or next provider request is released until the user grants applicable trust.

#### Scenario: Denied trust or closed review
- **WHEN** the user refuses nested instruction trust or the foreground channel closes
- **THEN** Kuru neither activates the new source nor executes a pending mutation or exposes the pending read result.

#### Scenario: Headless path discovery
- **WHEN** a headless actor call reaches an unapproved nested source
- **THEN** Kuru returns a bounded trust-required diagnostic and leaves the source and target result inactive, without waiting for interactive input.

#### Scenario: Persistent nested approval on another launch
- **WHEN** the user persistently approves one path-qualified complete nested manifest and starts a later turn at the same root
- **THEN** the root-only preflight remains valid, only a call entering that checked path set can use the nested approval, and changed source bytes or identity require fresh review.

#### Scenario: Revocation and concurrent approval
- **WHEN** `trust revoke` removes an existing workspace record while another session is reviewing a nested persistent choice against that record
- **THEN** the stale reviewer cannot silently recreate the revoked approval or drop another session's newer grant; it must review the current state again.

#### Scenario: Permission denial precedes discovery
- **WHEN** tool permission denies a target beneath a nested instruction directory
- **THEN** that target does not cause the nested source to be captured, reviewed, or activated.

### Requirement: Progressive project prompt sources retain complete trust binding

Automatic project skill metadata and effective custom-command bytes SHALL contribute typed source-bound claims to the initial exact-root manifest. Selected project skill body/reference bytes SHALL contribute typed claims only to a complete supplemental manifest, without changing the approved base. The existing once/persist/deny flow and private v2 root record SHALL govern each new effective supplemental source set, and persistent publication MUST compare the exact pre-review generation under the root lock. Revocation or base reapproval during review MUST invalidate a pending publication. User-config entries remain caller authority and MUST NOT bless unrelated repository instructions, hooks, MCPs or tools.

#### Scenario: Later skill body cannot borrow metadata approval
- **WHEN** a project skill's metadata is approved at startup but its body is selected later
- **THEN** the metadata approval alone does not authorize the body's prompt bytes or any reference, and the complete supplemental manifest is reviewed before exposure.

#### Scenario: Revoked approval while waiting
- **WHEN** a persistent skill review starts against one approval generation and that root approval is revoked or replaced before publication
- **THEN** publication fails without restoring the old base or selected material.
