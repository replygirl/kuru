## MODIFIED Requirements

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
