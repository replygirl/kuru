## Context

The application currently ships native macOS and Linux archives. Important
boundaries assume Unix: filesystem identity and modes, process groups and
signals, supervisor lifetime pipes, executable names, shell execution, PTY
fixtures, archive replacement and installation scripts. Compiling those paths
out would leave incomplete Windows behavior, particularly for memory ownership
and cleanup.

The active `native-platform` change independently implements and verifies the
small OS primitives library with compiled Rust fixtures. This product change
consumes it; the package's Windows CI is not product support evidence. The
active `dolt-memory` change establishes the full Dolt storage behavior. The
active `embedded-runtime` change puts a verified, target-specific Dolt archive
inside every Kuru executable. This change depends on all three and extends those
contracts to `x86_64-pc-windows-msvc`. Downloading Dolt on a user's first chat is
not an implementation option.

## Goals / Non-Goals

**Goals**

- Native x64 Windows 10 version 1809 or newer, including Windows 11; native
  MSVC builds and required `windows-2025` CI, without WSL or a Unix shell.
- The same chat, framework, persistent memory, dream, undo, tools, authentication
  and connector behavior, with native paths, commands and terminal interaction.
- Preserve private storage, exclusive writers, cancellation, process ownership,
  bounded I/O, durable activation and verified executable replacement.
- One installed `kuru.exe` sufficient for offline demo and persistent memory;
  no separate MSVC redistributable installation; native mise and PowerShell
  binary installation before source instructions.
- Reuse domain logic and meaningful fixtures behind narrow OS boundaries.

**Non-Goals**

- Windows ARM, Windows before 1809, WSL-only support, new providers or frameworks.
- Supporting filesystems that cannot satisfy the required ACL, reparse, stable
  identity and same-volume durable move operations by weakening those checks.
- A new daemon users manage, database downloads at runtime, Python/Bun tooling,
  a PowerShell application framework, or a generic platform plugin system.
- Changing release strategy inputs, retry semantics, notes generation, existing
  immutable assets or the final position of Pages publication.

## Decisions

### 1. One narrow platform package, with explicit safe contracts

Consume `packages/kuru-platform` from the hard-blocking `native-platform` change,
which is independently buildable without memory or delivery. The primitive
implementations and package-only tests belong to that foundation; this change
owns consumer adoption and actual product acceptance below.
It owns process/IPC, handle identity, private filesystem creation/validation and
durable publication primitives used by consumers. Domain packages retain SQL,
archive format, authentication, shell authorization and update policy. Platform
operations accept `Path`/`OsStr` and typed options, not lossy strings or opaque
`std::process::Command` introspection.

Windows API bindings may use a current pinned Rust Windows bindings dependency.
Any necessary unsafe FFI is confined to the Windows implementation in this
package, with documented ownership, buffer lifetime, thread-safety and error
invariants plus RAII handles. The existing unsafe-code prohibition remains in
all other crates; no workspace-wide relaxation. Unix implementations preserve
their existing guarantees. Package-owned mise tasks expose verification.

Rejected: scattered `cfg` branches and duplicate helpers in each domain,
because subtle differences would split ownership rules. A broad OS framework
would add abstraction beyond the actual consumers.

### 2. Explicit process creation and separate supervisor responsibility

Use an explicit spawn description containing executable, argument vector,
environment policy, working directory, stdio, owned inherited handles and
lifetime policy. For commands and probes that need tree ownership, create the
process with native `CreateProcessW`, extended startup information and both
`PROC_THREAD_ATTRIBUTE_JOB_LIST` and `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` in the
same call. The Job denies breakaway and has an explicitly owned lifetime. There
is no spawn-then-assign gap, even when the parent dies during creation. Native
completion processing verifies the Job's active process count reaches zero;
root-child exit or an arbitrary completion packet alone is insufficient.
[Microsoft documents these creation attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).

The memory supervisor is a different, trusted role. It must survive parent
endpoint loss long enough to close SQL, stop Dolt and release the writer lease
in that order. It is not placed in a parent kill-on-close Job that could prevent
that cleanup. It begins with a bounded initial configuration deadline (the
existing five-second contract); missing configuration or lifetime-channel EOF
exits without starting Dolt. After configuration, the supervisor owns the Dolt
Job and retains the lease through graceful shutdown, bounded escalation, root
reaping and authoritative descendant quiescence. Parent loss is a cleanup
request over the lifetime channel. Startup failure and parent death before and
after acknowledgment must each have a bounded, native regression. This
exception is limited to Kuru's own supervisor, not arbitrary tool commands.

Keep the SQL supervisor generic over asynchronous input/output. Add a platform
adapter for its local lifetime channel and spawning rather than fork SQL
supervision. On Windows use overlapped named pipes with first-instance creation,
a current-user ACL and connected-peer process identity checks. Client and
server reads, writes and connect waits are cancellable; no blocking pipe reader
can keep Tokio shutdown alive. Only the explicit handle allowlist is inherited,
including during concurrent launches. Stale PID records never confer authority
to kill a process.

Use standard Windows argument quoting for ordinary executable argv and
case-insensitive environment-key comparison compatible with Windows, including
duplicate rejection and the environment entries required by system processes.
Keep native command-shim handling separate from ordinary argv. Do not recreate
an opaque `Command` through guessed environment removals or lossy formatting.
[Rust describes the distinct Windows argument rules](https://doc.rust-lang.org/std/process/#windows-argument-splitting).

Rejected: process-wrapper spawn-then-assign Jobs, root-exit-only waits, blocking
Tokio child stdio for supervisor lifetime reads, blanket Jobs over the supervisor,
or numeric PID-based recovery. Each leaves a concrete ownership or shutdown gap.

### 3. Private storage and publication use held Windows handles

Create private cache, database, credential and staging roots with a protected
DACL for the current user at creation. Allow legitimate Dolt-created descendants
to inherit that private ACL only through a validated private chain with all
grants still owner-only; do not require the protected bit on every descendant.
Validate owner, DACL and file type before use; an existing permissive or
ambiguous object fails closed rather than being silently adopted. Do not equate
a Windows readonly bit with Unix private mode bits. Administrators' OS-level privileges are outside the
same-user application privacy boundary; ordinary other-user access is not.

Reject every reparse point at protected boundaries, including junctions; reject
unsafe hardlink counts. Retain handles while validating ancestors and leaves,
locking, reading a source or publishing a destination. Use volume identity plus
the full file identifier and revalidate the path-to-handle relationship. Open
flags explicitly allow or deny delete sharing according to the operation, so a
lock cannot silently continue protecting a replaced pathname. NTFS is the
required native CI filesystem; another filesystem is usable only if these same
primitives succeed, without a weaker fallback.

Flush written files and publish on the same volume with explicit overwrite or
no-clobber semantics and the Windows durable-move operation. Do not emulate this
with cross-volume copy/delete, a Unix directory `sync_all`, or an unqualified
`tempfile::persist`. Pre-publication failure preserves prior bytes. An API error
after a possible move is an uncertain outcome: reconcile held identity and the
durable receipt before reporting or retrying, retaining recoverable old bytes.
Do not promise unchanged destinations on every post-move error or claim that
Windows write-through proves POSIX-equivalent power-loss durability.
Directory activation, marker publication and recovery retain the stable
lifecycle lock through the final durable transition.
[Windows file opening and sharing](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [flush semantics](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
define the handle-level operations; their composed recovery behavior requires
native tests.

SQLite migration remains a one-time compatibility operation through SQLite's
backup API. On Windows, validate and hold source/parent/WAL/SHM identities and
deny replacement while the backup executes: a SQLite `NOFOLLOW` flag alone is
not a Windows reparse defense. Preserve source files and accepted WAL content,
then run the same verified staged activation and reopen checks.

Rejected: chmod-style privacy checks, symlink-only rejection, a 64-bit file-ID
assumption for every filesystem, or silently reduced fsync guarantees.

### 4. Extend the embedded bundle with a verified Windows payload

Add the official full Dolt Windows amd64 ZIP to the memory-owned pin manifest
introduced by `embedded-runtime`. Keep one version and authoritative hashes for
compressed archive, executable and LICENSES; bundle preparation belongs to the
memory mise task and uses the host-built delivery helper. Cargo reads verified
local build inputs only. Cross-compilation selects `TARGET`, never executes a
foreign binary and cannot silently embed the host engine.

At runtime, validate the embedded archive and extract only the fixed executable
and license into a private staged cache. Pinning, strict size bounds, fixed
membership and per-file digests apply before the executable is probed or used.
Do not call a network downloader or search PATH for an engine. Explicit supported
binary overrides remain deliberate configuration with the same version check;
the default always uses the embedded payload. An empty cache and offline mode
must work from the installed executable alone.

ZIP validation must inspect the complete physical member inventory before an
API that deduplicates names. Enforce local/central record agreement, exact names,
types and bounded compressed/expanded sizes; reject duplicate names, links,
encryption and unsupported or inconsistent metadata. Decode fixed members to
bounded writers, never broadly extract. Reuse a small strict ZIP boundary where
appropriate for the outer Kuru ZIP; do not duplicate the whole decoder in each
consumer. The observed upstream ZIP has four entries: root directory, LICENSES,
bin directory and `dolt.exe`. Record exact measured pin metadata in the manifest,
not copied constants throughout platform code.
[Dolt's upstream release](https://github.com/dolthub/dolt/releases/tag/v2.3.3)
is the artifact source. Some ZIP APIs
[store members by name](https://docs.rs/zip/latest/src/zip/read/zip_archive.rs.html#44-61),
so decoder enumeration alone is not evidence of duplicate rejection.

Rejected: a separate user-installed engine, first-use download, a Windows-only
database implementation, or a platform helper depending on memory/delivery.

### 5. Native consumer behavior preserves existing authority

Use native user configuration/data defaults (`APPDATA` and `LOCALAPPDATA`),
preserving existing explicit configuration and data-dir overrides. Do not require
HOME on Windows. Treat drive and UNC paths as filesystem paths before generic
URL parsing. Tests cover spaces, Unicode and case-insensitive environment keys.

Resolve executable commands and supported Windows command shims explicitly.
MCP and native authentication continue using argument vectors without accidental
shell interpretation. Shell-tool requests deliberately use the stock native
PowerShell executable with profiles disabled and predictable input/output and
exit-status behavior. Shell strings remain intentional shell code subject to
the existing authority checks. No new provider or authentication method is
introduced. Native file tools preserve project confinement, including reparse,
hardlink, alternate-stream, device-name and trailing-dot/space aliases under
Windows path rules.
[Microsoft's file-naming rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
and [Rust executable resolution](https://doc.rust-lang.org/std/process/struct.Command.html)
explain why Unix string substitutions are insufficient.

Retain the current TUI layout, animation and interaction design. Share renderer
and scenario intent while using native ConPTY/input adapters for chat,
navigation, resize, focus, cancellation and restoration. Actual Windows console
state must be restored on normal and error exits. Unix termios assertions remain
on Unix; Windows assertions must inspect real Windows state. ConPTY establishes
the Windows 10 1809 floor.
[CreatePseudoConsole's requirements](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
are the compatibility baseline.

Rejected: WSL/Git Bash as a runtime prerequisite, copying POSIX terminal bytes
blindly, suppressing unsupported paths with `cfg`, or adding unrelated UI work.

### 6. Windows delivery preserves validation and observable completion

Centralize target metadata: Rust triple, native executable name and archive
format. Add `kuru-<version>-x86_64-pc-windows-msvc.zip` with exactly three flat
regular members (`kuru.exe`, `LICENSE`, `README.md`) and a checksum sidecar. Keep
the four existing tar.gz names and layouts unchanged. Full Kuru executables
include their Dolt bundle; packaging must not accidentally omit it.

Shipping Windows builds explicitly select the MSVC target and static CRT.
Target-specific flags must not affect host build scripts or proc macros. Native
CI inspects PE imports of both Kuru and the embedded Dolt executable and permits
only operating-system DLL dependencies; a separately installed redistributable
must not be needed to start the shipped application.

Add package-owned `support/install.ps1` with a thin conventional entrypoint,
compatible with stock Windows PowerShell 5.1. Retain explicit version, target,
release-base and install-dir options plus documented environment precedence.
The default resolves latest's bounded checksum manifest once and then uses an
explicit version URL. Bound manifest, compressed archive, expanded data and
final executable output (existing limits unless measured shipped bytes require
a justified change). Verify hashes and the exact ZIP inventory before fixed
stream extraction into a private same-destination stage. Do not execute the
candidate. Cancellation or validation failure preserves the prior executable;
ordinary inactive destinations use verified durable replacement. The bootstrap
does not pretend it can replace a separately locked running executable.

The native updater must additionally handle its own loaded Windows image. Use
a private same-directory verified candidate, retained old-image identity and a
trusted current-version cleanup helper. Publication success means the installed
path already refers to the validated new bytes; cleanup of the displaced loaded
image may wait for its owning process to exit. If the OS requires more than one
rename, record the recovery phases durably, restore the old path on handled
failure and recover an interrupted transition before another update. Do not
describe two renames as a single atomic swap. Neither a downloaded candidate
nor a batch script performs validation or cleanup. No reboot or elevated
installation is required. Helper creation, parent-exit observation and its own
cleanup have explicit ownership and bounded failure behavior; native tests must
prove the actual sequence before selecting a helper implementation.
[The upstream self-replace implementation discussion](https://docs.rs/self-replace/latest/self_replace/)
describes the loaded-image rename/unlink distinction; it does not establish our
privacy, durability or failure-preservation guarantees.

Mise uses the GitHub backend with the native ZIP and exposes `kuru.exe` for both
exact installation and activation. Source installation goes through the
prepared app mise build task, including embedded target preparation, with no
shadow language workspace. README order is mise, platform binary bootstrap,
then source; guides describe ordinary available software.

Rejected: unverified candidate execution, a batch/sleep/reboot updater, trusting
generic ZIP extraction, or adding a separate Pages workflow to publish docs.

### 7. Native CI is an acceptance gate, not a cross-compile claim

Add a required native `windows-2025` x64/MSVC job to CI and the aggregate gate.
Use package-owned mise tasks with native commands and compiled Rust fixtures.
Exercise platform ownership, real Dolt, runtime, CLI, connector/auth, ConPTY,
PowerShell installer and loaded-image updater behavior. Unix-only mechanics may
remain platform-scoped only when their Windows contract has an executed native
counterpart. Report actual counts and failures; an unavailable prerequisite is a
failure for a required test, not a skip counted as success.

Keep the existing workspace coverage threshold of at least 90 percent and
meaningful Unix checks. Collect native Windows coverage for platform-specific
code and record its measurement/limitations explicitly; never infer it from
Linux coverage or remove product code to satisfy a threshold. Establish the
working Windows instrumented task before calling the native quality gate
complete. Measure task duration and bound real-process waits by observable
handshakes, not arbitrary sleep-based retries.

Add the fifth native release build from the exact prepared version commit. Run
its packaged executable with an empty engine cache, no external Dolt/compiler,
and offline runtime access, then verify persistent demo/reopen. Publication
requires all target artifacts and the unchanged immutable-asset checks.
Strategy-only dispatch, conventional-commit bump/recovery, Communiqué notes,
and docs build/deploy after publication retain their existing graph. Fresh
required checks on the actual candidate commit and native release artifact
evidence are necessary; this planning document is not such evidence.

## Operational surface

Run natively on Windows 10 1809+ x64; CI uses windows-2025 and explicit MSVC
shipping targets. Kuru carries the target's pinned full Dolt engine and uses
native OS DLLs only. No container, WSL, external database installation or runtime
download is required. The parent and trusted supervisor communicate through a
private local named pipe; Dolt retains its existing authenticated loopback-only
SQL endpoint and bounded pool. No public bind address or remote SQL access is
introduced. Existing operation, startup, output and shutdown bounds remain
enforced, including the five-second unconfigured supervisor deadline.

Fixtures generate isolated credentials and paths and require no provider secrets.
Normal authentication uses existing configured providers and private stores.
The release continues using its existing scoped credentials; adding a target
does not create a new release secret or a second publication route.

## Integration contract

The platform package supplies owned native handles, local IPC, process creation
and filesystem publication; it does not own application routes, SQL schemas,
mounts or model behavior. Memory retains stable part/session/revision IDs,
candidate branches, expected-base promotion and migration verification. Runtime
and TUI keep the same async memory API and in-memory render boundary. Delivery
owns archive policy and the single target catalog; memory owns its embedded
engine manifest and mise preparation. No dependency from platform back to either
consumer is allowed.

Native integration fixtures are compiled Rust peers with explicit argv, private
environment and parent-controlled handshakes. ConPTY and PowerShell fixtures
exercise their real Windows interfaces. Existing Unix fixtures remain where
their mechanics are platform-specific; shared behavioral expectations must also
execute on Windows. No provider SDK or protocol schema is changed.

## Risks / Trade-offs

- **Native process and filesystem code needs unsafe Windows API calls** → Keep
  a narrow audited module, safe typed surface, owned handles and real failure
  regressions; no unsafe-code waiver in consumers.
- **A supervisor may be killed too early or outlive a dead parent** → Separate
  bounded startup, lifetime EOF and supervisor-owned Dolt Job responsibilities;
  kill the fixture parent at each handshake phase and prove lease retention and
  eventual active-zero cleanup.
- **Headless Windows console control differs from an interactive terminal** →
  Prove private-console graceful Dolt shutdown on the native runner, with bounded
  Job escalation preserving the lease through quiescence. If that mechanism
  cannot meet the contract, resolve it before application rollout.
- **Named pipes or files may be replaced or connected by another process** →
  First-instance private pipes, peer identity, validated private ACLs, held file identity
  and explicit handle inheritance; adversarial native fixtures.
- **Embedded bundles increase build and archive sizes** → Measure final Windows
  bytes and decompression bounds in packaging tests; changes to bounds require
  evidence and equivalent negative tests.
- **Loaded-image replacement has multiple durable phases** → Explicit recovery
  records, retained old bytes, native interruption tests and accurate success
  wording; no claim of atomicity stronger than the primitive.
- **Windows CI is the only current native execution host** → Keep all native
  verification rows unchecked until actual hosted evidence exists; retain
  isolated fake credentials and no developer/provider data in fixtures.

## Implementation evidence still required

The design selects contracts; it does not assert the unrun Windows primitives
already work. The first implementation tasks must prove overlapped pipe
cancellation and peer identity, console-mediated graceful Dolt exit on a
headless runner, full handle identity and same-volume durable publication, and
the loaded-image replacement recovery sequence. A failure blocks the dependent
implementation rather than authorizing a weak fallback. There are no deferred
product choices about runtime downloads, platform scope or release ordering.
