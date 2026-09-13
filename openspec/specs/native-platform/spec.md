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
using validated same-volume native operations with required flushing. It MUST
NOT copy/delete across volumes, overwrite an unchecked destination or silently
omit platform durability steps. Pre-publication rejection MUST preserve old
bytes. Possible post-move errors MUST expose uncertainty for caller identity and
receipt reconciliation rather than imply no mutation occurred. Domain receipts
and recovery policy SHALL remain with callers.

#### Scenario: Occupied or unsafe target
- **WHEN** a new-only target exists or a replacement target fails validation before the move
- **THEN** publication fails and the existing target remains byte-for-byte unchanged.

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
`ECHILD` or another ownership-invalidating error, signal the original group and
root only while the root is unreaped, consume signal authority when destructive
attempts begin, and permit only read-only group observation after reap.

The Unix boundary MUST NOT claim that a process group provides atomic
owner-death containment or controls processes that deliberately escape it.
Windows cancellation and close MUST observe root reaping and actual owned-tree
quiescence before reporting completion. A Unix ordinary shell result MUST
observe root reap and group absence; a bounded cleanup failure MAY return an
unconfirmed result only while an independent owner retains the child and
associated capability until later confirmation. Stale PIDs MUST NOT authorize
termination on any platform.

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
- **THEN** platform signals the original group then root while retaining the unreaped standard child, reaps the exact root status, and performs no destructive numeric signal afterward.

#### Scenario: Unix child ownership cannot be established
- **WHEN** non-reaping observation reports `ECHILD` or another ownership-invalidating error before destructive cleanup begins
- **THEN** platform permanently disarms signal authority, reports the distinct ownership state, and never blindly signals the saved numeric group or root.

#### Scenario: Unix observation remains interrupted beyond caller return
- **WHEN** bounded `EINTR` exhausts an observation attempt while the exact root remains owned and unreaped
- **THEN** platform sends nothing and retains anchored authority so a later valid observation can perform the one destructive transition; once that transition begins, no later poll can signal again.

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
