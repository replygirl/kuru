## Context

The product Windows plan needs shared OS primitives, but its database and bundle
dependencies are not prerequisites for testing those primitives. This change
adds only an independently buildable `packages/kuru-platform` library, native
Rust fixtures and its own mise/CI checks. Consumer adoption remains in
`native-windows`.

## Goals / Non-Goals

**Goals:** small safe APIs, held ownership across namespace/process operations,
explicit failure outcomes, shared filesystem semantics on Unix and Windows,
Windows process/IPC primitives and real native fixture evidence.

**Non-Goals:** SQL, Dolt, migrations, embedded payloads, ZIP/tar decoding, shell or
authentication policy, updater orchestration, product UI/CLI integration and
claims of Windows application support. No generic backend/plugin framework or
additional language workspace. Do not introduce a Unix arbitrary-command
supervisor or strengthen existing Unix consumer process/IPC guarantees here.

## Decisions

### 1. Concrete package surface and contained FFI

Expose checked directory/file operations, opaque full file identity and explicit
publication policy across supported platforms. Windows-only exported modules
provide `NativeSpawnSpec`, owned process lifetime operations and a private local
asynchronous channel. Accept native `Path`/`OsStr`; callers retain
domain IDs, receipts, credentials and policy. Return ordinary owned files and
handles where useful rather than wrap all byte I/O in a framework.

Use exactly pinned workspace dependencies and package-owned mise tasks. Safe
Unix APIs remain safe; necessary Windows FFI lives in a small reviewed module
with RAII allocation/handle owners and documented borrowing/error invariants.
The platform crate may choose its explicit deny-by-default lint configuration
and locally allow reviewed Windows FFI; all other crates keep workspace
`unsafe_code = forbid`. Do not relax the workspace prohibition.

Rejected: per-consumer duplicates or a whole generic OS layer. Neither improves
the few concrete operations required by the consumer audit.

### 2. Checked directories retain native identity

Use a directory guard holding its validated path and ancestor handles. Child
operations accept one literal component, rejecting separators, dot components,
NUL and Windows ADS/device/trailing-dot-space aliases. A checked regular-file
open rejects links/reparse points and unsafe link counts; the low-level metadata
operation may report a hardlink's identity so a caller can detect aliasing.

Windows opens use explicit sharing and OPEN_REPARSE_POINT, with directory
semantics where needed. Inspect actual handle type, reparse status, link count,
volume and complete 128-bit file ID. A movable guard permits delete sharing for
legitimate publication; a pinned guard denies it for source-name retention.
Verify held/name identities after acquiring locks and around publication. Unix
uses corresponding no-follow relative operations and dev/inode identity.

Create the requested private root with an owner-only protected DACL at creation
on Windows and private modes on Unix. Require the process-token owner, persistent
ACL capability and owner-only grants. Descendants may inherit an owner-only ACL
through the validated private chain; do not require a protected bit on every
descendant. Existing unsafe ownership/permissions fail without repair. Structural
ancestors are checked but are not required to be user-owned. Standard `File`
locking remains the primitive; never replace or unlink a held lock.

The supported boundary is a private directory controlled by its user, not a
sandbox against a malicious process running as that same user. Do not claim
that Win32 absolute opens plus identity rechecks remove every same-user race.
Unsupported filesystems fail explicitly; native NTFS is required evidence, and
other filesystems receive no unmeasured compatibility claim.

Rejected: DOS readonly-as-privacy, canonicalizing away junctions, pathname hashes,
truncated file IDs, chmod-after-permissive-create and silent ACL repair.

### 3. Publication distinguishes rejection from uncertainty

Offer new-only and replace-checked-regular-file publication, plus moving a
closed directory to a new name. Validate source/destination/parents, flush a
writable candidate and use same-volume native publication. Unix uses relative
rename/no-replace plus parent synchronization; Windows uses explicit native
write-through moves without COPY_ALLOWED. Do not substitute directory
`sync_all` on Windows or a cross-volume copy/delete fallback.

A pre-move rejection leaves the old object intact. A native error after a move
may be uncertain: preserve held identities and return enough outcome context
for callers to reconcile their own receipt before retrying. The library does
not invent database/update receipts or promise unchanged bytes after every API
error. Do not claim POSIX-equivalent power-loss proof from a Windows flag.

Rejected: unconditional `tempfile::persist`, blind retries and domain recovery
hidden inside filesystem helpers. Callers know their accepted transaction and
retain responsibility for reconciliation.

### 4. Spawn intent is explicit and ownership precedes execution

`NativeSpawnSpec` contains executable, argv, environment policy, cwd, stdio,
explicit inherited handles and lifecycle selection. Ordinary executable argv
uses native quoting; environment comparison on Windows is case-insensitive with
duplicate rejection and correct UTF-16 ordering. Shell strings and `.cmd` policy
remain consumer responsibilities. Do not introspect an opaque `Command` or copy
ambient secrets to reconstruct it.

Windows owned trees are created in one `CreateProcessW` call with
JOB_LIST/HANDLE_LIST startup attributes; deny breakaway and retain kill-on-close
ownership. Never spawn and assign afterward. Root reaping and the Job's actual
zero-active-process state both matter; arbitrary completion packets or root exit
alone do not prove quiescence. Drain owned output without leaving blocking Tokio
readers. The Windows Job proves owner-death cleanup through native fixtures.
Existing Unix consumers retain their current process-group and lifetime-channel
implementations; this change adds no generic Unix Job equivalent or new
arbitrary-command lifetime supervisor. No consumer entrypoint is wired here.

Operations expose bounded cancellation and explicit asynchronous close/wait;
destructors initiate owned cleanup but never falsely report completed quiescence.
A trusted lifetime-driven owner is distinct from an ordinary kill-on-close tree:
the package provides mechanics, while the future memory consumer decides when
its supervisor must outlive a caller to clean up. Numeric PID records alone do
not confer authority to terminate another process.

Rejected: process-wrapper spawn-then-assign Jobs, blanket parent Jobs over a
future database supervisor, root-only waits, and sleep/retry cleanup scripts.

### 5. Local IPC is private and cancellation-safe

Use private first-instance overlapped named pipes on Windows with process-token
ACLs and connected-peer identity checks. Existing Unix local socket/pipe adapters
remain in their consumers and are not refactored by this foundation. Channel
connect/read/write/close are asynchronous and bounded by caller cancellation.
Explicit native handle allowlists avoid inheritance into unrelated children.
Do not convert a blocking Windows anonymous pipe into a Tokio wrapper and assume
that cancellation can stop its worker thread. No TCP listener or public endpoint
is introduced by this package.

Rejected: globally named permissive pipes, endpoint-name secrecy as identity,
blocking lifetime readers, and transport methods that own domain SQL framing.

### 6. Independent native proof

Compile fixture executables in this package, using private plans and observable
handshakes for argv/env capture, process trees, output stalls, lifetime channels
and filesystem contention. They require no shell, Dolt, network service or
provider credential. Keep subprocess environment changes in children and retain
actual exit/status diagnostics; no global test-runner environment mutation.

Add a required package-only windows-2025 x64/MSVC CI job and aggregate dependency.
Run this package's format/lint/test/coverage mise tasks without building memory,
TUI or bundle inputs. Unix native jobs execute shared filesystem contracts;
Windows process/IPC tests execute only on their actual native host. Preserve
the repository's 90% workspace line-coverage floor and measure the new package's
native code honestly; a missing required fixture/API fails, not skips. The job
name and documentation explicitly say platform package, not Windows product.

Rejected: cross-compilation as proof, Windows no-op test exclusions, dependency
on product fixtures and a successful CI label concealing zero native tests.

## Operational surface

The new runtime processes are isolated compiled fixtures and owned child trees.
The Windows API floor is Windows 10 1809+ x64; CI uses windows-2025. Unix runs
native macOS/Linux fixtures. There are no public binds, containers, required
secrets, runtime downloads or database connections. Each operation has bounded
startup/I/O/shutdown waits and diagnostic process state; callers control their
own concurrency budgets. Pin build tools and OS bindings through existing mise
and Cargo lock ownership.

## Integration contract

The library depends only on Rust/native OS facilities. It has no routes, mounts,
SQL schema, product IDs or external service SDK. Consumers receive safe owned
handles, native paths, typed spawn options and observable publication/process
outcomes. Fixture programs implement the same public API without a backend mock.
The dependent `native-windows` change adopts those contracts and must still prove
real database, terminal, installer and update behavior before claiming support.

## Risks / Trade-offs

- **Unsafe native ownership mistakes** → Isolate FFI, immediately wrap handles,
  document lifetimes and exercise partial-construction/cancellation paths.
- **APIs differ despite similar names** → Require native identity, ACL,
  publication, owner-loss and overlapped-I/O fixtures; unsupported guarantees
  fail explicitly rather than degrade.
- **Post-move errors are uncertain** → Return checked outcome context, retain
  identities and require caller reconciliation instead of assuming no change.
- **Package CI could be mistaken for product support** → Keep CI/task labels and
  docs scoped to the library; retain all three product hard dependencies and its
  separate uncompleted acceptance ledger.
