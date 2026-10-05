# native-platform Specification

## Purpose
Provide shared, checked native filesystem, process ownership and asynchronous
IPC primitives while preserving domain package boundaries and auditable handle
lifetimes.

## Requirements

### Requirement: Independent native primitives package

The repository SHALL provide `packages/kuru-platform` as an independently
buildable Rust library owning native filesystem, process and local IPC mechanics.
It MUST NOT depend on application, database, embedded engine, archive or updater
policy. Safe APIs SHALL contain any necessary audited Windows-only FFI while
consumer crates retain their unsafe-code prohibition.
Filesystem APIs SHALL have native Unix and Windows implementations. New process
and IPC APIs MAY be Windows-only modules; existing Unix consumer process/IPC
behavior MUST remain unchanged by this foundation.

#### Scenario: Package-only native build
- **WHEN** package checks run with no Dolt, bundle input or application build
- **THEN** the library and its compiled native fixtures build and exercise their actual OS contracts.

### Requirement: Checked private filesystem objects

Checked operations MUST validate actual object type, full native identity,
ownership, privacy and safe link counts under retained directory/file handles.
Protected paths MUST reject symlinks and every Windows reparse point. New private
roots MUST receive private permissions at creation; Windows roots MUST have a
protected owner-only DACL. Descendants MAY inherit owner-only grants through a
validated private chain. Existing unsafe objects MUST fail without silent repair.

#### Scenario: Valid inherited private descendant
- **WHEN** a descendant inherits only its validated private root's owner grants
- **THEN** checked operations accept it without requiring a protected DACL bit on that descendant.

#### Scenario: Unsafe existing object
- **WHEN** a fixture presents permissive/null/foreign ACLs, a junction, an unsafe hardlink or a special file at a protected boundary
- **THEN** the operation rejects it without accessing private content through the alias or changing foreign data or permissions.

#### Scenario: Name changes while ownership is held
- **WHEN** a fixture attempts to substitute an object name after handle validation or lock acquisition
- **THEN** pinning prevents the change or full held/name identity checks reject further protected operations before publication.

### Requirement: Explicit native publication outcomes

Publication SHALL provide new-only and checked-regular replacement semantics
using validated same-volume native operations with required flushing. Windows
checked replacement MUST retain the validated source, destination parent and
existing destination identities through one handle-relative native replacement;
an existing destination handle MUST remain bound to its replaced object while
later opens of the destination name resolve to the published source. Publication
MUST NOT copy/delete across volumes, overwrite an unchecked destination or
silently omit platform durability steps. Pre-publication rejection MUST preserve
old bytes. Possible post-move errors MUST expose uncertainty for caller identity
and receipt reconciliation rather than imply no mutation occurred. Domain
receipts and recovery policy SHALL remain with callers.

#### Scenario: Occupied or unsafe target
- **WHEN** a new-only target exists or a replacement target fails validation before the move
- **THEN** publication fails and the existing target remains byte-for-byte unchanged.

#### Scenario: Retained Windows replacement target
- **WHEN** checked Windows replacement retains an open validated destination handle through publication
- **THEN** one handle-relative native operation publishes the exact staged source, the retained destination handle still identifies and reads the replaced object, and later destination opens identify and read the published source.

#### Scenario: Native move has an uncertain result
- **WHEN** the move may have occurred before an error was returned
- **THEN** retained identity and outcome information allow the caller to determine actual state before retrying, without an automatic destructive rollback.

### Requirement: Explicit native spawn semantics

The Windows `NativeSpawnSpec` SHALL express executable, native argv, environment policy,
working directory, stdio, inherited handles and lifetime selection. Ordinary
argv MUST retain its boundaries without shell interpretation. Windows environment
handling MUST honor native case-insensitive keys and reject ambiguous duplicates.
The package MUST NOT reconstruct intent through lossy `Command` introspection.

#### Scenario: Native argument and environment round trip
- **WHEN** a compiled fixture receives empty arguments, quotes, spaces, Unicode, metacharacters and explicit environment keys
- **THEN** it observes exactly the intended values and no unrelated command or ambient secret is introduced.

### Requirement: Owned process tree lifecycle

Windows owned process execution MUST establish ownership before executing child
code, supplying Job and inherited-handle attributes atomically at creation
without a spawn-then-assign interval. Job ownership MUST enforce cleanup when
its owner dies.

The platform MAY provide a narrow Unix fresh-process-group owner for the
built-in shell, but it MUST own spawn, the standard child, root identity,
signal-before-reap transition, reaping, and absence observation together. It
MUST NOT expose a child or numeric identity that a safe caller can independently
reap and later signal. It MUST observe ownership immediately before signalling,
retry a bounded `EINTR` without consuming authority, permanently disarm on
`ECHILD` or another ownership-invalidating error, and signal the original group
and root only while the exact root is unreaped. The immediate initial transition
MUST remain one-shot. A nonblocking pre-reap step MAY repeat an ordered sweep
only after a fresh exact-root ownership observation and before the caller's
existing absolute deadline. It MUST retain at most one read-only membership
worker, launch no listing for a running root, exclude only the retained root
from listed group membership, and keep descendant zombies or unavailable
listings pending. Snapshot admission MUST use bounded spawn-lock acquisition
and the absolute inspection deadline. Worker cancellation/expiry MUST retain
helper cleanup ownership; late results MUST NOT cause signals. After reap or
disarm, only read-only group observation is permitted.
Completion waiting MUST retain its receiver across caller cancellation and use
the caller's existing poll/deadline; it MUST NOT join an unfinished worker or
replace the pre-reap step's result classification and fresh signal authority.

The Unix boundary MUST NOT claim that a process group provides atomic
owner-death containment or controls processes that deliberately escape it.
Windows cancellation and close MUST observe root reaping and actual owned-tree
quiescence before reporting completion. A Unix ordinary shell result MUST
observe root reap and group absence; a bounded cleanup failure MAY return an
unconfirmed result only while an independent owner retains the child and
associated capability until later confirmation. Stale PIDs MUST NOT authorize
termination on any platform. Pre-reap readiness MUST NOT replace the final
post-reap absence and output gates or allocate another cleanup allowance.

#### Scenario: Owner disappears during startup
- **WHEN** a fixture owner using the owner-loss contract exits at a creation or startup handshake boundary
- **THEN** its child tree cannot persist beyond the bounded owned cleanup and unrelated processes remain untouched.

#### Scenario: Root exits before its descendant
- **WHEN** a root exits while an owned grandchild is active or retains output
- **THEN** root status alone does not complete the tree wait; descendants and owned output are quiescent before completion is reported.

#### Scenario: Concurrent child inheritance
- **WHEN** concurrent children have distinct explicit inherited-handle lists
- **THEN** neither receives the other's private handles and independent lifetime closure remains observable.

#### Scenario: Unix signal authority is consumed before reap
- **WHEN** a fresh Unix shell group reaches natural or forced cleanup
- **THEN** platform signals the original group then root while retaining the unreaped standard child, freshly authorizes any bounded repeated sweep, reaps the exact root status, and performs no destructive numeric signal afterward.

#### Scenario: Unix child ownership cannot be established
- **WHEN** non-reaping observation reports `ECHILD` or another ownership-invalidating error before any destructive cleanup attempt
- **THEN** platform permanently disarms signal authority, reports the distinct ownership state, and never blindly signals the saved numeric group or root.

#### Scenario: Unix observation remains interrupted beyond caller return
- **WHEN** bounded `EINTR` exhausts an observation attempt while the exact root remains owned and unreaped
- **THEN** platform sends nothing and retains authority so a later valid observation within the existing deadline may perform its permitted transition or repeated sweep.

#### Scenario: First Unix group sweep misses a live descendant
- **WHEN** a ready descendant survives the initial sweep in the original group while the root exits
- **THEN** pre-reap settlement observes the member and performs a real freshly authorized second sweep before exact root reap; a descendant zombie remains pending.

#### Scenario: Unix inspection is queued past cleanup expiry
- **WHEN** worker admission or the shared spawn lock remains unavailable until the caller's deadline
- **THEN** no late snapshot or repeated tree signal is admitted, caller polling stays nonblocking, and helper cleanup remains owned without reporting ready.

#### Scenario: Unix cleanup is cancelled and resumed
- **WHEN** an async cleanup poll is cancelled while a membership worker is pending
- **THEN** the owner retains and consumes that single job on resume and never uses a late result after expiry, reap or disarm to signal the tree.

#### Scenario: Unix membership completes before the next caller poll
- **WHEN** the retained read-only worker completes helper cleanup within the caller's deadline
- **THEN** its retained completion wake advances the existing loop without waiting for the full poll timer, and only a finished worker is joined before readiness is classified.

#### Scenario: Coverage disposes an expired pending Unix owner
- **WHEN** final coverage supervision returns unconfirmed with an exited root and a pending membership helper
- **THEN** final disposal reaps only the exact exited root, keeps the unconfirmed result, and leaves helper cleanup under its read-only continuation without signalling.

### Requirement: Cancellable private local IPC

The package SHALL provide Windows local asynchronous channels with explicit
private access and expected-peer identity validation. These channels MUST use
first-instance overlapped endpoints and owner-only ACLs. Connect, read, write
and close MUST support bounded cancellation without a stranded blocking reader
preventing runtime exit. The channel SHALL remain independent of SQL or product
message schemas.

#### Scenario: Stalled channel is cancelled
- **WHEN** a peer stalls connection, sends a partial frame or stops reading and the caller cancels
- **THEN** the operation ends within its bound, owned resources close and the runtime exits without a hidden blocked worker.

#### Scenario: Wrong endpoint or peer
- **WHEN** an existing endpoint or unexpected peer attempts the handshake
- **THEN** validation fails before private fixture payload transfer and no other process's endpoint is adopted.

### Requirement: Native package verification

Package-owned mise tasks and a required native Windows x64/MSVC CI job SHALL
exercise the real filesystem, process and IPC primitives using compiled Rust
fixtures. Shared filesystem checks MUST also execute natively on Unix. Required unavailable
prerequisites MUST fail rather than skip; measured native coverage and test
counts MUST be recorded honestly while preserving the workspace coverage floor.
Passing package checks MUST NOT be described as Windows Kuru product support.

#### Scenario: Native platform regression
- **WHEN** a required Windows package test or quality check fails
- **THEN** its required job and aggregate gate fail even if existing product jobs pass.

#### Scenario: Foundation passes before product integration
- **WHEN** all platform package checks pass while the product Windows change is pending
- **THEN** records describe verified OS primitives only and retain the product's separate Dolt, embedding and application acceptance gates.

### Requirement: Complete one-shot Unix child pipes

The retained Unix fresh-process-group owner SHALL accept the caller's declared
policy for standard input, output, and error (pipe, null, or inherit) and SHALL
itself create every declared pipe; it MUST NOT reconstruct stdio intent through
`Command` introspection, and std MUST NOT create a stdio pipe for an owned
spawn. The owner SHALL permit a safe caller to take each declared pipe exactly
once without exposing the standard child or its numeric process identity.
Taking pipes MUST NOT transfer root observation, signal, reap, or post-reap
authority away from that owner. The child's ends MUST be closed in the parent
before spawn returns.

#### Scenario: Session takes all configured pipes

- **WHEN** a connector creates a retained Unix child with piped stdin, stdout, and stderr
- **THEN** it can take each pipe once for asynchronous framing and draining while the original owner remains solely responsible for ordered process-group cleanup

#### Scenario: Undeclared pipe is not available

- **WHEN** a caller declares a stream null or inherited and later takes that stream
- **THEN** the owner reports that the stream was not piped and its cleanup authority is unchanged

### Requirement: Checked directory-tree removal

The platform SHALL provide a consuming checked directory-tree removal primitive
for a caller-held private `Directory`. It MUST retain and verify the root
identity, reject symlink and Windows reparse descendants, expose rejected versus
uncertain native removal outcomes, and never authorize removal of a replacement
that later occupies the former name. Domain inventory, retry, and receipt policy
remain with the caller.

#### Scenario: Replaced or linked removal root
- **WHEN** a caller-held directory name is replaced or a descendant is a symlink
  or reparse point before removal
- **THEN** the primitive rejects without traversing or removing the replacement
  or link target.

#### Scenario: Native removal uncertainty
- **WHEN** native directory removal may have happened before an error is
  observed
- **THEN** the outcome retains the original identity and enough location detail
  for the caller to reconcile without a destructive automatic retry.

### Requirement: Private successive-client service IPC

The native platform package SHALL provide a local listener that accepts successive independent clients on Unix and Windows without granting service, database or process authority to the transport. Unix socket creation and connection MUST require a checked owner-private directory and an owner-private socket of the expected type; a listener MUST NOT adopt or remove an occupied name. Windows named-pipe instances MUST retain a private local-only DACL, and cancelling an accept MUST preserve the pending native operation so a later accept can complete safely. Releasing a listener SHALL remove only its own checked Unix socket name. The consuming service MUST separately authenticate every connection; native permissions do not isolate processes running as the same OS user.

#### Scenario: Successive independent clients
- **WHEN** two clients connect to one native listener in succession
- **THEN** both obtain usable byte channels and the first client's lifetime does not block the second

#### Scenario: Cancelled Windows accept
- **WHEN** a Windows service listener's bounded accept times out before a client connects
- **THEN** a later accept can complete the original pending connection without discarding kernel-owned buffers or stranding the listener

#### Scenario: Unsafe Unix endpoint
- **WHEN** a Unix caller supplies a public parent, a non-socket endpoint or a socket with group/other access
- **THEN** binding or connecting rejects the endpoint without adopting it as private service IPC

#### Scenario: Occupied or replaced Unix name
- **WHEN** the chosen Unix socket name already exists or the bound name is replaced before listener drop
- **THEN** binding rejects the occupied name and cleanup does not remove the replacement

### Requirement: Concurrent owned Unix spawns keep pipes private

The fresh-process-group owner SHALL create its stdio pipes close-on-exec and
start the child under one process-wide platform spawn lock, and the bounded
process snapshot SHALL start its child under the same lock, so its std pipes
are created under the lock; no other platform spawn can then copy a
descriptor table that holds a pipe end without close-on-exec. Where the OS
creates pipes close-on-exec atomically the owner MUST use that call. The lock
MUST be private to the platform and, in product builds, held only across pipe
creation and child creation, never across caller code, waiting, or I/O on the
child; only test builds may run their own seams under it.
The platform MUST NOT claim isolation from spawns or descriptor creation that
bypass it: an unrelated legacy spawn can still inherit an owned pipe end in its
own window, and an owned child can still inherit descriptors that other code
creates without atomic close-on-exec. On std's fork path a legacy spawn that
inherits an owned spawn's exec-error pipe stalls that spawn, and with it every
later platform spawn, until the legacy child exits.

#### Scenario: Concurrent owned children reach end of file independently

- **WHEN** two owned children that read standard input to end of file start concurrently on separate threads and each parent drops its input end at once
- **THEN** each child exits on its own end of file within the existing bound while the other child is still running

#### Scenario: Forced window between pipe creation and close-on-exec

- **WHEN** one owned spawn is paused after creating a pipe and before marking it close-on-exec, and a second owned spawn starts on another thread
- **THEN** the second spawn waits for the platform spawn lock, the first child reaches end of file once its parent drops its input end, and the second child holds none of the first child's pipe ends
