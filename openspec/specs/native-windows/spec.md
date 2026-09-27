# native-windows Specification

## Purpose

Provide native Windows chat, persistent Dolt memory, tools and terminal behavior
with compiler-free installation and recoverable running-executable updates.
Preserve private filesystem identity, owned process cleanup and database
reconciliation through the shared platform boundary, and require native tests,
measured coverage and actual packaged-runtime acceptance alongside Unix support.

## Requirements

### Requirement: Native Windows application

Kuru SHALL support x64 Windows 10 version 1809 or newer and Arm64 Windows 11 or
newer through native MSVC executables for `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc`. Chat, framework selection, sessions, preferences,
memory, dreams, undo, tools, authentication and connectors MUST retain their
existing behavior on both targets without requiring WSL, a Unix shell, a
compiler, an MSVC redistributable installation or a separately managed database.
Shipping builds MUST link the appropriate static CRT for their target without
changing host build-script/proc-macro settings. Fresh bootstrap installation
MUST select the target from the host's native machine architecture, never from
the architecture reported by an emulated shell's or parent process's
environment, and MUST fail closed on any other machine. Update MUST keep the
target of the installed executable, so an x64 executable running under
emulation on Arm64 continues to update to x64; moving such an installation to
Arm64 is outside this requirement. Recovery of an interrupted update MUST accept
the recorded helper's own Windows target even when it differs from the native
machine. A Windows target SHALL be documented as supported only after native
memory, process cleanup, terminal, installation and update checks have passed
on that architecture.

#### Scenario: Fresh native installation
- **WHEN** a user launches the installed executable in an isolated Windows user environment on either supported architecture
- **THEN** version and help work, an offline demo persists and reopens its conversation, and no external Dolt executable or runtime download is required.

#### Scenario: Native executable dependencies
- **WHEN** native CI inspects the shipped Kuru and embedded Dolt PE imports for a Windows target
- **THEN** only operating-system DLL dependencies are present and no separately installed MSVC redistributable is required.

#### Scenario: Bootstrap ignores an x64 architecture environment
- **WHEN** the PowerShell bootstrap runs on an Arm64 machine with `PROCESSOR_ARCHITECTURE` reporting `AMD64`
- **THEN** it determines the native machine through the operating system rather than the environment, selects the `aarch64-pc-windows-msvc` archive and requires an ARM64 PE32+ executable, or fails closed when asked for a target that does not match the native machine.

#### Scenario: Explicit target mismatch
- **WHEN** the PowerShell bootstrap is given an explicit `-Target` that differs from the target of the host's native machine, including an x64 target on an Arm64 machine
- **THEN** it fails closed before any download with a message that names the native machine's target, and installs nothing.

#### Scenario: Emulated x64 installation updates
- **WHEN** an installed x64 `kuru.exe` running under emulation on an Arm64 machine updates, or the bootstrap recovers its interrupted update
- **THEN** the update installs the x64 target and recovery accepts the recorded x64 helper, without replacing the executable with one of a different machine type.

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
and recorded receipts before a retry or a claimed publication outcome. When a
bundled-runtime activation move returns access denied and fresh checked
observations prove that the source retains the held identity and the destination
is absent, Windows activation SHALL retry for no more than two seconds while
retaining the same private stage, source authority and cache lock. Any other
error or observed state MUST fail immediately without retrying publication.

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

#### Scenario: Runtime activation is proven not to have moved
- **WHEN** Windows reports access denied while activating a verified bundled runtime and checked observations show the held source identity remains at its original name while the destination name is absent
- **THEN** Kuru retries the same activation only within the bounded two-second recovery period while retaining the private stage and cache lock, and success still requires the destination to acquire the verified source identity.

#### Scenario: Runtime activation outcome is not proven safe to retry
- **WHEN** activation reports another error, the source identity changes or cannot be observed, or the destination is occupied or cannot be observed
- **THEN** Kuru performs no activation retry, preserves the original typed error and recoverable stage, and does not report publication success.

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

CI SHALL require native Windows MSVC tests on every supported Windows
architecture, currently x64 and Arm64, for platform, database, runtime, CLI,
connector, terminal, installation and update behavior. A missing required
prerequisite MUST fail rather than skip. A runner label that the native test
partition table does not declare for the requested mode MUST fail every
partition and merge of that call, and therefore the aggregate gate, rather than
fall into a Unix or x64 branch. Platform-scoped Unix tests SHALL retain their coverage and
have actual Windows counterparts for shared behavior. Native Windows coverage and
its limits MUST be reported honestly per architecture alongside the unchanged
workspace quality threshold, including any test artifact excluded by name on one
architecture and the recorded reason.

#### Scenario: Windows-specific regression
- **WHEN** a native test, required prerequisite or Windows coverage check fails on the candidate commit on either Windows architecture
- **THEN** the required Windows job for that architecture and the aggregate CI gate fail even if every other job passes.

#### Scenario: Unknown Windows runner label
- **WHEN** a native-tests call names a Windows runner label that the partition table does not declare for its mode
- **THEN** every partition and the merge of that call fail with `no <mode> partition set is declared for <label>` before any test runs, and the aggregate gate fails on their results instead of accepting Unix-path results for the label.

### Requirement: Staged Windows release acceptance

Each release SHALL exercise its exact staged ZIP for every Windows target in the
release catalog on a native runner of that architecture before public GitHub
release promotion. The acceptance check MUST route the real mise GitHub backend
through an isolated loopback fixture serving simulated release metadata and those
exact candidate bytes, start with empty Kuru data and engine caches, run an
offline demo conversation, resume the same session, inspect memory status and
history, list sessions, and verify the extracted embedded engine and licenses
against the checked-out authoritative asset manifest for that target. The
staged check MUST also run the previous-release updater rule for that target:
when a published stable release carries the target's archive, the greatest such
release's own updater MUST install the staged candidate; when none does, the
check MUST record "no predecessor for <target>" with the releases it inspected
and pass. This staged check MUST NOT be claimed as an actual public download; the
package-owned published verifier remains available as a separate
post-publication diagnostic per target.

#### Scenario: Cold staged package persists and reopens
- **WHEN** the ordinary mise installation path consumes the exact staged release ZIP for a Windows target with the demo provider and an empty offline engine cache
- **THEN** it extracts the bundled engine, completes and resumes one durable session, reports the resulting memory state and history, and lists both turns without downloading another engine

#### Scenario: Staged native cleanup is uncertain
- **WHEN** any owned fixture or application process cannot be confirmed stopped before isolated state cleanup
- **THEN** native acceptance fails and public release promotion remains blocked

#### Scenario: Staged bytes differ from the candidate
- **WHEN** the ZIP served to mise or its checksum differs from the complete candidate artifact selected for publication
- **THEN** acceptance fails rather than substituting rebuilt or public bytes

#### Scenario: First release for a target
- **WHEN** no published stable release lists the target's core archive and no published `SHA256SUMS` names it
- **THEN** staged acceptance prints "no predecessor for <target>" with the inspected versions, skips no other check, and passes

#### Scenario: Listing and manifest disagree
- **WHEN** a published release's asset listing lacks the target's archive but its `SHA256SUMS` names it, or the reverse
- **THEN** staged acceptance fails rather than treating that release as having no predecessor
