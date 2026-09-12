## Context

Configuration loading currently merges files for several consumers, while memory
preferences and runtime construction can cause later reads and activation. The
feature spans the core loader, native held-directory mechanics, TUI command
startup, authentication, and connector process/protocol setup. The approved
boundary and requirements are in the proposal and `workspace-trust` delta.

## Goals / Non-Goals

**Goals:**

- Make the reviewed configuration object identical to the activated object.
- Bind durable approval to the object and authority values that were reviewed.
- Keep trust policy at the application boundary while reusing platform checks.

**Non-Goals:**

- Git-root inference, parent/child approval inheritance, a generic permission
  language, or a memory-backed trust store.
- A Unix spawn primitive that atomically binds a child cwd to a held directory.
- A claim that approval confines a same-user process or protects private files
  from an approved shell.

## Decisions

### Exact canonical `-C` root is the trust subject

The CLI canonicalizes and retains the same workspace directory used for project
identity before automatic ancestor discovery. A Git repository root, nearest
configuration directory, or parent trust scope is rejected because each changes
the existing non-Git project boundary or makes unrelated roots implicit.

### Core owns an immutable snapshot and leaf provenance

`kuru-core` retains parsed layers, merged pre-preference data, explicit local
patches, every CLI override (including `allow_write`, `allow_shell`, and
`no_dream`), and final source provenance for authority leaves.
The TUI asks core for a canonical manifest rather than reconstructing policy
from serialized TOML. Re-reading and hashing whole files is rejected: it loses
recursive-map leaf provenance and permits post-approval configuration swaps.

### Approval records store digests, not review text

The manifest separates canonical full-value claim hashing from redacted display.
An explicit `trust approve` or the pre-TUI **approve this configuration** choice
reviews and persists the complete current manifest only. The record stores only
schema/root identity, the complete manifest digest, sorted claim digests, and
audit metadata under checked private `<data-dir>/trust/workspaces`; claim
digests are audit data, never independent or partial grants. A matching complete
record may satisfy a command's applicable subset. Any authority addition,
removal, or value change changes the complete manifest and invalidates that
record globally. Storing raw configuration, rendered text, or records in Dolt
is rejected because either can duplicate secrets, expand an approval before
memory preflight, or couple trust to project history.

The digest also binds the full path identity of each contributing automatic
source through private unambiguous encoding. Safe display labels are bounded
and may truncate, so they cannot serve as source identity. Moving an effective
authority contribution to another source invalidates approval; ordinary edits
at the same source path do not. Filtered command reviews retain every source
contributing to the claims they display.

### Activation is selected per command before constructors run

The TUI derives the command's claims from one matrix and resolves approval before
opening memory, credentials, providers, `ToolHost`, MCP or a harness. A
one-invocation flag approves only the invoking command's applicable subset and
never writes state; narrow commands cannot silently persist or broaden an
approval. Explicit CLI and explicit-local values count only for their final
leaves; ordinary ancestor settings require no approval. A full always-on prompt
is rejected because account and inspection commands must not activate unrelated
authority.

### Configuration diagnostics are sanitized at the snapshot boundary

Snapshot loading converts read, parse, type, and validation failures to a
bounded escaped source label, a coarse category, and parser-provided line and
column when available. It does not expose error chains, TOML excerpts, field
values, or raw paths with terminal controls. Reusing `anyhow` context or TOML's
rendered parser error is rejected because either can quote MCP arguments,
environment values, or a malformed secret-bearing value.

### `config` omits saved preferences

`MemoryStore::open`, including its read-only option, creates private directories
and locks and provisions Dolt; it is not an attach-only inspection API. Therefore
`config` prints parseable TOML from the immutable snapshot without saved project
mode/model/effort preferences and emits a truthful bounded omission notice (a
TOML comment or stderr). It creates, migrates, provisions, and leases nothing.
Adding a new memory inspection API or config flag is rejected as broader than
this preflight feature. Ordinary runtime startup still loads saved preferences
from the retained snapshot after applicable preflight.

### Authentication retains fixed-route behavior

ChatGPT account commands construct their fixed native route without consulting a
workspace-selected Responses environment name. Responses availability checks and
catalog/completion requests wait for the route tuple claim. Treating every auth
command as provider construction is rejected because it reads an unapproved
secret name without a product need.

### Platform exposes mechanics only if current retained-directory APIs cannot express revalidation

The core/TUI retains a `kuru-platform::fs::Directory` and uses a narrow public
self-revalidation operation before trust decisions and cwd-based child launches.
If existing checked APIs already provide that operation, no new platform surface
is added. A trust-specific platform API is rejected because root policy, store
location and manifests belong above the platform boundary.

### Revalidation detects observed replacement; it does not close the Unix cwd race

Connectors revalidate immediately before a shell or stdio MCP pathname-based
spawn. A changed root fails before that launch; existing file tools keep their
capability-opened handles. Claiming atomic cwd binding, adding a broad Unix
process supervisor, or calling this a sandbox is rejected because none follows
from a pre-spawn identity check.

### Retain the approved root through configured spawns

The TUI owns one pinned retained workspace directory before ancestor parsing.
It passes that same capability into `ToolHost` and an injected approved host
into `Harness`; both clone it only for configured shell and stdio-MCP launch
paths. Those paths revalidate the original guard immediately before use and
must not replace it with an independently reopened workspace guard after
approval. A capability-root open remains permissible when bracketed by the
original guard's revalidation. A named retained-root `ToolHost` constructor and
compatible explicit-config convenience constructor are permissible; runtime
must not depend on the platform crate.

## Operational surface

Trust resolution runs in the local CLI process before the TUI alternate screen,
provider construction, memory provisioning and configured protocol setup. It
does not add a listener, container contract, remote secret, connection pool, or
binary/architecture requirement. The only persisted state is the checked local
approval record beneath the selected data directory; noninteractive commands
return a bounded error instead of opening a prompt.

## Integration contract

There is no new external SDK, mount, route, or wire protocol. Core exposes an
immutable snapshot and redacted/canonical manifest to the TUI; the TUI passes a
retained checked workspace capability to connectors only after preflight.
Approval-record and manifest schemas are versioned local data contracts and use
typed fixture constructors rather than raw private-state files. Platform exports
only a domain-neutral retained-directory revalidation operation if the existing
API cannot express it.

## Risks / Trade-offs

- **[Complex precedence retention]** → Construct final configuration only from
  the retained snapshot and prove explicit/local/preference precedence and a
  post-preflight file swap in integration tests.
- **[Secret disclosure in diagnostics]** → Give manifest claims typed redacted
  renderers, bound all display fields, and test controls, URL queries and MCP
  environment values directly.
- **[Cross-package startup regressions]** → Keep the command matrix in one TUI
  preflight boundary and use real no-child, no-socket, no-secret-read and
  no-memory-mutation fixtures for every activation family.
- **[Platform overclaim]** → Cover native replacement detection on Unix and
  Windows, document the remaining Unix pathname race, and avoid adding a
  platform API unless the retained handle needs an exposed revalidation method.
