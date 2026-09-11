## Context

The application currently ships native macOS and Linux archives. Important
boundaries assume Unix: filesystem identity and modes, process groups and
signals, supervisor lifetime pipes, executable names, shell execution, PTY
fixtures, archive replacement and installation scripts. Compiling those paths
out would leave incomplete Windows behavior, particularly for memory ownership
and cleanup.

The archived `native-platform` change independently implements and verifies the
small OS primitives library with compiled Rust fixtures. This product change
consumes it; the package's Windows CI is not product support evidence. The
archived `dolt-memory` change establishes the full Dolt storage behavior. The
archived `embedded-runtime` change puts a verified, target-specific Dolt archive
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

`TrustedSupervisor` omits Kuru's new Job; it does not escape an enclosing Job
inherited from an external caller. Creator-process EOF and external whole-tree
termination have different contracts. The former must leave the supervisor alive
to perform ordered cleanup. The latter is a crash path: after authoritative
whole-tree quiescence, reopen committed state and reconcile activation receipts;
do not report a graceful shutdown receipt or assume a dead supervisor can clean
up. Keep the creator-loss fixture's creator outside a test-owned kill Job and
terminate only its retained process handle. A separate enclosing-Job fixture
tests containment and subsequent crash recovery. Incompatible Job assignment
fails startup without an uncontained fallback; no breakaway, elevation or
enclosing-policy mutation is introduced. See [Windows Job inheritance](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [nested Job limits](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs).

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

Keep flushing newly written payloads in `publish_file`. Add a narrow checked
rename operation for unchanged existing files, retaining the same identity,
privacy, same-volume, write-through and uncertain-outcome checks without flushing
the unwritten source. Loaded old executables and retired endpoint records may
be held read-only; Windows
[`FlushFileBuffers` requires write access](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).
This operation
does not establish durability for unflushed payload writes and must not replace
the publication path for new candidates or receipts.

Windows cannot move a directory while descendant data-file handles remain open,
even when those files allow delete sharing. Use an external Windows lifecycle
namespace, `<data_dir>/memory/lifecycles`, shared by active, staging and interrupted
stores. `ServerOptions` and the private supervisor request carry an explicit
`lifecycle_root`; Windows requires it, while Unix retains its existing in-directory
lock layout for compatibility. Direct server callers must reuse the same root
for each open/quiescence of a physical store. Reject a namespace inside the store
tree by checked native identity, rather than a case-sensitive path prefix.

Derive the external lock's fixed filename from the held store directory's full
native identity: volume plus all 128 file-ID bits, encoded from a documented
24-byte representation. Add only a safe `FileIdentity::to_bytes` accessor to the
platform boundary; lifecycle routing remains memory policy. Native identity is
available before domain metadata exists, so lock acquisition precedes reading or
creating mutable domain identity. Revalidate the directory and lock identities
after acquisition. Retain a typed `LifecycleLease` containing the directory,
external root and lock guards through verified process quiescence and directory
publication. The key survives same-volume stage/active/interrupted renames; a
copied or recreated directory has a different native identity. Persistent unlocked
lockfiles need no deletion. The existing parent project lock still serializes
initial stage enumeration and selection.

Before moving a stopped stage, close all descendant SQL, record, snapshot and log
handles, retaining only the movable directory guards and external lease. Replace
the lease's directory guard with the successful move's guard after checking full
identity equality. Before replacing a record, close an old destination read handle;
the native primitive test demonstrates that retaining it can prevent replacement.
Keep candidate write/flush ownership and reconcile `Uncertain` using identity and
domain receipts. Retire endpoint records through checked publication into private
staging, then remove only the disposable retired object.
[Windows file opening and sharing](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [flush semantics](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
define the handle-level operations; their composed recovery behavior requires
native tests.

SQLite migration remains a one-time compatibility operation through SQLite's
backup API. On Windows, validate and hold source/parent/WAL/SHM identities and
deny replacement while the backup executes: a SQLite `NOFOLLOW` flag alone is
not a Windows reparse defense. Preserve source files and accepted WAL content,
then run the same verified staged activation and reopen checks.

Hold each present WAL/SHM/journal sidecar and the source parent through backup and
source-connection close. Pinned handles prevent replacement, while SQLite's
transactions and backup API provide consistency with legitimate concurrent
writes. Check absent sidecars before opening and newly present sidecars afterward
under the private parent; this is not a custom VFS or a hostile-same-user sandbox.
Preserve DB/WAL data and accepted transactions while permitting SQLite's normal
ephemeral SHM coordination. The pinned Windows VFS can create or rebuild SHM even
with a read-only database connection; do not promise byte-identical SHM or use
`immutable=1`. Retain a writable checked snapshot handle through destination
connection close, then flush, hash and publish the validated snapshot. Native
fixtures cover committed WAL, present/missing SHM and source replacement attempts.

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

The shared ZIP boundary belongs to `packages/kuru-archive`, a small pure-Rust
codec package consumed by memory and delivery. It takes bounded archive bytes
and an explicit expected-member policy, validates the physical local and central
records before opening the decoder, and copies individual verified entries to
caller-provided bounded writers. It performs no filesystem, process or network
operations and knows no runtime versions, release targets or installation paths.
Memory owns Dolt's exact four-member inventory and payload hashes; delivery owns
Kuru's three-member inventory and release checksum. Neither domain depends on the
other, and archive-format mechanics do not enter `kuru-platform`.

Use the current pinned `zip` 8.6.0 with default features disabled and only its
flate2 deflate support enabled; the existing pinned flate2 backend is retained.
The accepted ZIP subset is single-disk, non-ZIP64, non-encrypted stored/deflate
entries with complete local sizes and CRCs, no data descriptors, no comments,
and no leading, inter-member or trailing unaccounted data. Allow only the
observed well-formed central NTFS timestamp extra field for the upstream Dolt
archive; Kuru's own writer emits no extra fields. Fixed member names are ASCII.
The audited low external attributes are `0x8010` for directories and `0x8020`
for files. Accept p7zip's documented `0x8000` POSIX-mode marker alongside the
ordinary directory/archive bits; reject other low bits and never apply those
attributes to output files. The Unix creator, exact type and pinned mode remain
authoritative. Kuru's writer explicitly selects the Unix creator on every host
for deterministic metadata; its directory entries may omit the optional DOS bit.
[7-Zip's attribute mapping](https://github.com/ip7z/7zip/blob/main/CPP/7zip/Archive/Zip/ZipItem.cpp)
documents that marker.
Reject duplicate physical names and local offsets, inconsistent types, size/CRC
disagreement, links and unsupported flags before decoding. The four audited Dolt
entries have zero flags and no local extra fields, so this subset accepts the
actual pinned payload without a compatibility fallback. Decoder tests include
the actual upstream archive and mutations of physical records, independently of
Windows execution. [Current ZIP feature/MSRV documentation](https://docs.rs/zip/8.6.0/zip/)
was checked on 2026-09-10; its Rust 1.88 floor is below the workspace toolchain.

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

Make connector RPC spawning asynchronous so native pipe setup and atomic owned
creation use the verified platform API. Route all four authentication actions,
including the CLI's separate device-login path, through one typed connector
action and its bounded owned lifetime. Preserve client-owned credential handling.
Capture and merge child environments case-insensitively with the existing native
comparison routine; never mutate the parent environment or log values. Resolve
bare commands only through usable entries in the effective child PATH, skipping
unusable entries while preserving later valid commands. Do not fall back to the
parent PATH or silently search the project. Explicit paths never trigger PATH
fallback. Add conventional EXE/COM/CMD/BAT extension resolution while leaving
unsupported file associations to a deliberately configured interpreter.

Batch shims require a small typed command-syntax variant inside the existing
native spawn boundary, with the OS system `cmd.exe` and a hardened batch encoder;
MSVCRT argv quoting is not a batch encoder. Preserve common explicitly configured
`cmd /c npx` MCP setups by resolving the literal target and forwarding its argument
vector. A single explicit post-`/c` source argument can intentionally express cmd
operators/built-ins/expansion; document its native outer-quote convention and do
not silently reinterpret ordinary arguments as shell source. Preserve current
Rust's evidenced NUL/CR/LF and command-size rejection behavior, with native tests
for percent expansion, quote/metacharacter combinations, unsupported script paths,
`/d`/`/s` forms and conventional forwarding shims. No legacy spawn fallback may
lose Job or handle-list ownership. Any adapted upstream encoder retains its source
attribution and license. [Rust's batch-argument handling](https://github.com/rust-lang/rust/blob/1.98.1/library/std/src/sys/args/windows.rs#L219)
and [batch security limitations](https://blog.rust-lang.org/2024/04/09/cve-2024-24576/)
inform the implementation; actual native fixtures establish support.

The explicitly authorized shell tool uses stock system PowerShell 5.1 with no
profiles and noninteractive text output, passing intentional source through its
UTF-16LE encoded-command option instead of cmd quoting. Test Unicode stdout/stderr,
explicit exits, terminating errors, failed native invocations and successful work
independently; do not append a stale `$LASTEXITCODE` that hides PowerShell failure.
All streams and waits keep their existing bounds and owned descendant cleanup.

Retain the current TUI layout, animation and interaction design. Share renderer
and scenario intent while using native ConPTY/input adapters for chat,
navigation, resize, focus, cancellation and restoration. Actual Windows console
state must be restored on normal and error exits. Unix termios assertions remain
on Unix; Windows assertions must inspect real Windows state. ConPTY establishes
the Windows 10 1809 floor.
[CreatePseudoConsole's requirements](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
are the compatibility baseline.

Use the current `portable-pty` 0.9.0 only in the Windows terminal test driver;
its child creation is serialized inside that test binary and does not replace
the application's owned process boundary. A native fixture sharing the ConPTY
console observes actual input/output modes before and after Kuru, and injects
focus records through a safe test-facing platform console API. The current
crossterm Windows event source reads `FOCUS_EVENT_RECORD`; Unix `ESC[O` bytes
alone are not evidence of native focus loss. Likewise its raw-mode disable path
sets standard input bits rather than restoring the original snapshot. Capture
and restore both original console modes through a small safe Windows platform
guard around the TUI lifecycle, including partial initialization failure. Keep
all necessary interop in the audited platform package and retain the consumer
unsafe prohibition. Native tests must observe mode restoration from outside the
application; renderer-only assertions cannot satisfy this contract.

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
The standalone bootstrap may use a minimal `Add-Type` Win32 bridge for checked
handles, private ACLs and write-through replacement. Stock PowerShell's .NET
Framework supplies this capability; no separate compiler, language project,
toolchain or runtime download is installed. Keep its native declarations and
handle lifetimes audited and exercise them in actual PowerShell 5.1 fixtures.
This bootstrap-only bridge is needed before downloaded Kuru can be trusted or
executed; all Rust consumers still forbid unsafe code and share `kuru-platform`.
Do not claim that `Add-Type` performs no compilation internally, or substitute
unproven managed-file durability for the native publication contract.
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
trusted current-version cleanup helper. Materialize that helper by copying the
held running executable into a private content-addressed helper cache, verifying
its bytes and identity before launching its explicit internal mode. Intentionally
retain this immutable cached image after helper exit, like the managed engine
cache; it must not launch another process to delete its own mapped executable.
No separate helper download, release asset or user installation is introduced.
Do not assume `current_exe()` still identifies the loaded image after another
instance updates that pathname. Pin the checked current file against writes and
renames while comparing its native NT name with the main module's mapped-file
name, then read helper bytes through that retained handle. Ambiguous or stale
instances must restart before updating. The mandatory native fixture renames
the loaded original and replaces its pathname with a byte-identical different
file: version or digest equality must not turn that substitution into acceptance.
The [mapped-file query](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getmappedfilenamew)
documents the API; the fixture must establish its required rename behavior on
the actual native runner. Drop the immovable guard before publication, retaining
the recorded full identity for reconciliation under the installation lock.
Publication success means the installed
path already refers to the validated new bytes; cleanup of the displaced loaded
image may wait for its owning process to exit. If the OS requires more than one
rename, record the recovery phases durably, restore the old path on handled
failure and recover an interrupted transition before another update. Do not
describe two renames as a single atomic swap. Neither a downloaded candidate
nor a batch script performs validation or cleanup. No reboot or elevated
installation is required. The helper owns the installation lock and durable
receipt, verifies publication before acknowledging it, and then waits on an
inherited duplicate of the actual parent process handle before retiring the
displaced image. Cleanup is bounded: failure retains an explicit pending receipt
and old bytes for checked recovery, without reversing an acknowledged publication.
Cached helper retention is intentional; temporary candidate and displaced-image
cleanup still require observed completion. An enclosing Job can terminate this
helper, so durable recovery remains required rather than assuming it escapes
that authority. Native tests must prove this complete sequence.

Keep the displaced loaded image as a unique sibling under its original
installation access policy, retaining its identity. A same-volume move does not
convert an inherited ACL into a private one. Keep a separately verified rollback
copy and the phase/identity receipt inside private staging; do not silently change
an existing installed image's ACL or move it into a private directory while
claiming that changed its permissions. Normal rollback first restores the retained
original object; the private copy preserves verified recovery bytes.
[The upstream self-replace implementation discussion](https://docs.rs/self-replace/latest/self_replace/)
describes the loaded-image rename/unlink distinction; it does not establish our
privacy, durability or failure-preservation guarantees.

Mise uses the GitHub backend with the native ZIP and exposes `kuru.exe` for both
exact installation and activation. Source installation goes through the
prepared app mise build task, including embedded target preparation, with no
shadow language workspace. README order is mise, platform binary bootstrap,
then source; guides describe ordinary available software.

The public source entrypoint disables mise's automatic task-tool installation
and repository setup hooks before its first mise invocation. The owning app
task installs only its explicit Rust build toolchain and prepared inputs. Scope
the entrypoint settings to the operation and restore the calling PowerShell
environment on success and failure. Native entrypoint fixtures must begin with
absent or opposing inherited settings; CI's global safeguards cannot stand in
for this fresh-user path.

Pre-merge mise acceptance runs the pinned native Windows mise executable and its
actual `github:replygirl/kuru` backend against controlled release metadata and
the candidate commit's genuine packaged ZIP. Use documented URL replacements
for the exact GitHub API base, this repository's release API paths and download
paths, followed by a fixture-local denial route for unexpected HTTP requests.
The separate API-base rule covers attestation requests constructed after URL
routing. Keep the normal public-backend selection/checksum/extraction behavior:
no custom backend, direct asset URL, asset pattern or preseeded installation.
Disable the versions host, isolate config/data/cache/state/home/system-config
paths and configuration discovery, and disable gh/git/netrc credential lookup
with no inherited tokens or credential command. All processes and HTTP bodies
remain bounded; assert the requests, selected Windows asset and installed binary
digest so an ambient executable or live fallback cannot satisfy acceptance.
[Mise's URL replacement contract](https://mise.jdx.dev/url-replacements.html),
[pinned GitHub backend](https://github.com/jdx/mise/blob/v2026.9.4/src/backend/github.rs)
and [attestation routing](https://github.com/jdx/mise/blob/v2026.9.4/src/github/sigstore.rs)
support this fixture without changing user installation behavior.

Include all five target filenames in simulated metadata and serve actual
packager bytes for the selected Windows asset. Verify API-digest success,
corruption rejection before activation, and genuine `SHA256SUMS` discovery when
the API digest is absent; exercise the browser-to-API download fallback inside
the fixture too. Keep provenance checks enabled and explicitly return empty
fixture attestations, not fabricated signatures. Run `mise use`, `mise which`
and `mise exec` with the exact version, then persistent offline demo/reopen from
an empty engine cache. Record that metadata is simulated and executable/backend
behavior is native; this does not prove publication, CDN access or provenance.

The main-only release workflow cannot publish this feature before its required
archive and merge. Therefore actual published Windows mise installation is a
separate post-publication operational check in `docs/release.md`, pending the
first authorized release containing the Windows artifact. Preserve the user
contract for published installs, and leave that operational result pending when
the candidate fixture passes. It is not a circular pre-merge prerequisite and
does not authorize a release dispatch or another publication entrypoint.

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

Full application update verification hashes the embedded executable repeatedly
before acknowledging publication. Observed native debug timings put this work
beyond the existing acknowledgment budget. Evaluate a version-specific Cargo
development-profile override for the pinned SHA-2 dependency, inherited by tests,
so its concrete compression loops are optimized while workspace code stays in
its ordinary instrumented development/test profile. Cargo requires this shared
profile declaration at the workspace root. Keep release settings, CPU dispatch,
all identity/content checks, output limits and process deadlines unchanged.
Compare identical preloaded bytes with the same compiler, instrumentation and
consumer optimization before and after the override, verify the digest against
an independent implementation, then require the unchanged actual Windows update
cases, full LCOV and shipping checks. Host measurements support this experiment;
they cannot establish native updater acceptance.

The existing packaged update scenario also observes normal helper coverage
emission. During its instrumented build, use an isolated filename prefix in the
runner's existing profile directory for the update command alone. Require
nonempty profiles from both distinct processes within the helper cleanup bound;
retain those files for the ordinary coverage merge. An explicitly selected
shipping binary is not instrumented and retains its separate runtime acceptance.

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

The independent primitives have passed native Windows tests. Their application
composition remains unverified: the first implementation tasks must prove actual
supervisor cancellation and peer identity, console-mediated graceful Dolt exit on
a headless runner, retained filesystem identity through domain transitions, and
the loaded-image replacement recovery sequence. A failure blocks the dependent
implementation rather than authorizing a weak fallback. There are no deferred
product choices about runtime downloads, platform scope or release ordering.
