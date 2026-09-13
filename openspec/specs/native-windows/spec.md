# native-windows Specification

## Purpose

Provide native Windows chat, persistent Dolt memory, tools and terminal behavior
with compiler-free installation and recoverable running-executable updates.
Preserve private filesystem identity, owned process cleanup and database
reconciliation through the shared platform boundary, and require native tests,
measured coverage and actual packaged-runtime acceptance alongside Unix support.

## Requirements

### Requirement: Native Windows application

Kuru SHALL support x64 Windows 10 version 1809 or newer through a native MSVC
executable. Chat, framework selection, sessions, preferences, memory, dreams,
undo, tools, authentication and connectors MUST retain their existing behavior
without requiring WSL, a Unix shell, a compiler, an MSVC redistributable
installation or a separately managed database. Shipping builds MUST link the
appropriate static CRT without changing host build-script/proc-macro settings.

#### Scenario: Fresh native installation
- **WHEN** a user launches the installed executable in an isolated Windows user environment
- **THEN** version and help work, an offline demo persists and reopens its conversation, and no external Dolt executable or runtime download is required.

#### Scenario: Native executable dependencies
- **WHEN** native CI inspects the shipped Kuru and embedded Dolt PE imports
- **THEN** only operating-system DLL dependencies are present and no separately installed MSVC redistributable is required.

### Requirement: Platform-owned OS boundary

Reused OS process, IPC, privacy, identity and durable publication primitives
SHALL belong to a small Rust platform package. Domain policy MUST remain in its
own package. Necessary unsafe Windows API calls MUST be confined to an audited
Windows module exposing safe owned-handle operations; other crates MUST retain
their unsafe-code prohibition.

#### Scenario: Native operation cannot establish a guarantee
- **WHEN** a required OS operation cannot establish ownership, private access, stable identity or durable publication
- **THEN** it fails with actionable diagnostics and preserves existing state rather than selecting a weaker fallback.

### Requirement: Atomic owned process creation

Owned command, probe and database process trees MUST enter their Job at native
process creation using an explicit Job attribute and inherited-handle allowlist.
Creation MUST NOT leave a spawn-then-assignment interval. Only configured
handles and environment values SHALL reach the child. Cancellation and shutdown
MUST establish root reaping and authoritative descendant quiescence before
releasing resources protected by that tree's lifetime.

#### Scenario: Parent dies during command creation
- **WHEN** a fixture owner dies at each creation or startup handshake phase
- **THEN** its owned child and descendants cannot remain running outside the Job and unrelated processes remain untouched.

#### Scenario: Root exits while a descendant retains output
- **WHEN** a command root exits but a descendant still owns a pipe or remains active
- **THEN** cleanup does not report quiescence until the descendant is stopped and the output handles are closed.

#### Scenario: Concurrent inherited handles
- **WHEN** two children launch concurrently with different private handle lists
- **THEN** neither child receives the other's lifetime or data handles and closing one lifetime endpoint is observable without waiting for the other child.

### Requirement: Supervisor preserves database ownership

The trusted memory supervisor SHALL use a bounded initial configuration phase
and a private asynchronous lifetime channel. It MUST exit without starting Dolt
when configuration is absent or the parent endpoint closes before startup.
After startup it SHALL own Dolt's Job and retain the writer lease until accepted
SQL operations are reconciled, shutdown completes, and the full database tree is
quiescent. Parent loss MUST request cleanup rather than kill the supervisor
before it can perform this sequence. A stale PID MUST NOT authorize termination.

#### Scenario: Caller exits while database work is accepted
- **WHEN** the caller disappears after an operation was accepted
- **THEN** the supervisor preserves the existing reconciliation contract, prevents another writer from overlapping it, and releases the lease only after verified database shutdown.

#### Scenario: Cold supervisor receives no configuration
- **WHEN** a parent dies before sending initial configuration
- **THEN** the supervisor exits within its bounded startup deadline without creating a running database or leaking a held writer lease.

### Requirement: Cancellable private Windows IPC

Supervisor lifetime communication SHALL use private overlapped I/O with
first-instance endpoint creation, a current-user access policy and connected-peer
identity validation. Connect, read, write and shutdown operations MUST be
cancellable without leaving a blocking reader that prevents runtime exit.

#### Scenario: Peer withholds the next frame
- **WHEN** the peer connects but never completes an expected frame and the operation is cancelled
- **THEN** the wait ends within its bound, owned handles close, the runtime can exit, and the memory shutdown sequence remains enforced.

#### Scenario: Another process occupies or connects to the endpoint
- **WHEN** a fixture presents a preexisting endpoint or a peer with the wrong process identity
- **THEN** startup rejects it before exchanging private configuration or database credentials.

### Requirement: Private Windows filesystem identity

Private roots MUST be created with a protected current-user DACL and validated
ownership. Descendants MAY inherit private ACLs only through a validated private
chain when every grant remains owner-only. Protected paths MUST reject all
reparse points and unsafe hardlinks. Validation, locks, migration and publication
MUST retain native handles and full volume/file identity where needed to prevent
pathname substitution. Unsupported filesystem guarantees MUST fail closed.

#### Scenario: Hostile existing private path
- **WHEN** an existing cache, state, credential, lock or staging path has permissive access, a reparse point or an unsafe hardlink
- **THEN** the operation rejects it without reading private content through the alias, changing ownership or replacing existing data.

#### Scenario: Lock pathname is replaced
- **WHEN** a fixture attempts to replace a lock or state pathname while its validated handle is held
- **THEN** replacement is denied or the identity mismatch is detected before a second writer or publication can proceed.

### Requirement: Durable Windows memory transitions

Windows SHALL preserve lossless SQLite backup migration, complete isolated dream
candidates, expected-base promotion, compensating undo, accepted-operation
reconciliation and staged activation recovery. Publication MUST use validated
same-volume durable operations under the stable lifecycle guard. It MUST NOT
use cross-volume copy/delete or silently omit required durability steps.
An error after a possible native move MUST be reconciled against held identity
and recorded receipts before a retry or a claimed publication outcome.

#### Scenario: SQLite WAL migration through native paths
- **WHEN** a legacy fixture with accepted WAL content is migrated from a path containing spaces and Unicode
- **THEN** all accepted content and stable identifiers survive in Dolt, the source remains available, and a failed activation can recover without a partial live database.

#### Scenario: Dream is cancelled or promotion conflicts
- **WHEN** a whole-dream candidate is cancelled or its expected base has advanced
- **THEN** live memory remains unchanged, the candidate follows existing recovery policy, and later conversation and preference writes are retained.

#### Scenario: Activation is interrupted
- **WHEN** the process is interrupted at a staged rename or marker publication boundary
- **THEN** reopening under the stable lifecycle guard recovers a valid recorded state without treating invalid or partial storage as an empty project.

#### Scenario: Publication returns an uncertain outcome
- **WHEN** a native operation reports an error after its move may have occurred
- **THEN** recovery checks actual identity and receipts before retrying or reporting success, preserving recoverable old bytes without assuming the destination stayed unchanged.

### Requirement: Embedded full Dolt on Windows

The Windows executable SHALL contain the verified full Dolt payload and license
for its target, using the companion embedded-runtime build contract. First-use
cache extraction MUST validate archive structure, exact membership, bounds and
digests before executing the engine. Default runtime behavior MUST NOT download
the engine, search PATH for one, or substitute another database implementation.

#### Scenario: Empty offline cache
- **WHEN** only the packaged Kuru executable is available, the engine cache is empty and runtime network access is unavailable
- **THEN** Kuru extracts its embedded Windows engine and license, completes a persistent demo conversation and reopens it successfully.

#### Scenario: Invalid embedded ZIP member
- **WHEN** an archive fixture contains duplicate or inconsistent records, unexpected members, a link, encryption or oversized output
- **THEN** validation rejects it before candidate execution or publication and preserves an existing verified cache.

### Requirement: Native paths and command semantics

Windows configuration and data defaults SHALL use native user directories while
preserving explicit overrides. Drive and UNC paths MUST remain paths. Ordinary
connector and authentication commands MUST preserve argument boundaries and
support documented native executable/shim resolution without implicit shell
injection. Shell tools SHALL use the native shell deliberately under the same
authority checks. File tools MUST enforce project confinement under Windows
case, reparse, alternate-stream, device-name and path-alias semantics.

#### Scenario: Native command arguments and environment
- **WHEN** a configured command receives empty arguments, quotes, spaces, Unicode, metacharacters and case-varied environment keys
- **THEN** the real native fixture observes the intended argument vector and environment, and unrequested commands do not execute.

#### Scenario: Confined file tool encounters a Windows alias
- **WHEN** a request uses a junction, alternate stream, reserved device or trailing-dot/space alias to access protected or external content
- **THEN** the existing file authority boundary rejects the request without changing external data.

### Requirement: Native terminal behavior and restoration

The Windows TUI SHALL preserve the existing interaction and animation design
through native console events and ConPTY. Normal exit, cancellation and error
paths MUST restore the console state they changed. Tests MUST exercise actual
terminal I/O, including focus-dependent rendering, rather than substitute only
buffer snapshots or Unix byte assumptions.

#### Scenario: Interactive Windows session
- **WHEN** a native ConPTY session enters chat, opens selectors, navigates, resizes, loses focus, cancels work and exits
- **THEN** the intended commands and persistent choices are observed, focus loss stops animation after the completed frame, and console state is restored.

### Requirement: Required native Windows verification

CI SHALL require native Windows x64/MSVC tests for platform, database, runtime,
CLI, connector, terminal, installation and update behavior. A missing required
prerequisite MUST fail rather than skip. Platform-scoped Unix tests SHALL retain
their coverage and have actual Windows counterparts for shared behavior. Native
Windows coverage and its limits MUST be reported honestly alongside the
unchanged workspace quality threshold.

#### Scenario: Windows-specific regression
- **WHEN** a native test, required prerequisite or Windows coverage check fails on the candidate commit
- **THEN** the required Windows job and aggregate CI gate fail even if every Unix job passes.

### Requirement: Actual published Windows installation acceptance

Each release SHALL exercise its actual published Windows package on a native
Windows runner in a fresh isolated user environment. The acceptance check MUST
start with empty Kuru data and engine caches, run an offline demo conversation,
resume the same session, inspect memory status and history, list sessions, and
verify the extracted embedded engine and licenses against the checked-out
authoritative asset manifest.

#### Scenario: Cold published package persists and reopens
- **WHEN** the ordinary mise-installed release runs with the demo provider and an empty offline engine cache
- **THEN** it extracts the bundled engine, completes and resumes one durable session, reports the resulting memory state and history, and lists both turns without downloading another engine

#### Scenario: Published native cleanup is uncertain
- **WHEN** any owned verifier or application process cannot be confirmed stopped before isolated state cleanup
- **THEN** the native acceptance check fails and does not report successful verification
