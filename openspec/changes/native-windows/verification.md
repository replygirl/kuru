## 1. Native process ownership and supervisor handoff [critical]

- [ ] 1.1 @integration (agent) Run compiled native fixture children and grandchildren while killing the owner at process creation/startup handshakes -> owned commands enter their Job atomically, no descendant escapes, and unrelated processes survive.
- [ ] 1.2 @integration (agent) Launch concurrent children with distinct inherited handles and a root that exits before its grandchild -> private handles do not leak and quiescence waits for the actual final process and output closure.
- [ ] 1.3 @regression (agent) Kill a real supervisor caller before configuration, during acknowledgment and during accepted SQL work -> startup is bounded, accepted work follows reconciliation, a second writer cannot overlap, and the supervisor releases the lease only after Dolt is quiescent.
- [ ] 1.4 @integration (agent) Exercise graceful shutdown and forced escalation on the headless Windows runner -> Dolt closes normally where cooperative and its Job reaches verified zero active processes before any protected resource is released.
- [ ] 1.5 @regression (agent) Separately terminate a retained creator process and an enclosing owned Job -> creator-only EOF permits supervisor cleanup, while whole-tree termination reaches verified zero active processes and subsequent reopen recovers committed state without claiming a graceful receipt or escape from enclosing authority.

## 2. Private IPC cancellation and identity [critical]

- [ ] 2.1 @integration (agent) Stall named-pipe connection, partial frame reads and blocked writes, then cancel and shut down Tokio -> every operation finishes within its bound without stranded blocking readers or leaked handles.
- [ ] 2.2 @integration (agent) Present an existing pipe name, wrong connected process and concurrent unrelated child -> first-instance/ACL/peer checks reject impostors before private configuration transfer and lifetime EOF is not held open by leaked handles.

## 3. Filesystem privacy, identity and durable transitions [critical]

- [ ] 3.1 @integration (agent) Create native permissive/null/unsafe inherited ACL fixtures, junctions, other reparse points and hardlinks at protected roots/leaves -> private operations reject them without changing foreign data; legitimate Dolt descendants with inherited owner-only grants remain usable.
- [ ] 3.2 @integration (agent) Attempt pathname substitution under a held lock and concurrent cache publication -> full handle identity prevents overlapping ownership; exactly one valid publication is retained.
- [ ] 3.3 @regression (agent) Interrupt file/directory activation at recorded rename/marker boundaries on native NTFS, including an uncertain post-move result -> stable lifecycle locking and identity/receipt reconciliation preserve a valid state, including failure preservation and no-clobber behavior without assuming every error means no move occurred.
- [ ] 3.4 @integration (agent) Contend on the external full-native-identity lifecycle lease across stage/active/interrupted directory moves, closing descendant data handles first -> the same physical store retains one lock key and exclusive ownership, a recreated directory has a distinct identity, and a missing/inside-tree Windows namespace fails before database startup.
- [ ] 3.5 @integration (agent) Rename an unchanged existing file held read-only, including a loaded-image fixture, through the checked native move operation -> identity and bytes survive the same-volume move without requiring write access, conflicting names preserve recovery evidence, and newly written candidate publication still flushes its payload.

## 4. Real Dolt behavior on Windows [critical]

- [ ] 4.1 @integration (agent) Run real storage/runtime suites with the embedded full Windows Dolt -> private histories, stable IDs, preferences, sessions, transaction rollback and close/reopen match the existing contract.
- [ ] 4.2 @regression (agent) Migrate SQLite fixtures with committed WAL content, present/missing SHM, Unicode/spaced paths and source replacement attempts -> backup is lossless, held identities prevent substitution, original DB/WAL and accepted transactions are preserved while normal ephemeral SHM coordination is allowed, and interrupted activation recovers correctly.
- [ ] 4.3 @regression (agent) Cancel or conflict a complete dream candidate, then exercise promotion, lost acknowledgment reconciliation and compensating undo after later chats/preferences -> no partial dream reaches live state and later writes survive undo.

## 5. Embedded bundle and ZIP validation [critical]

- [ ] 5.1 @e2e (agent) Run only the actual packaged Windows executable with an empty cache, no external Dolt/compiler on PATH and memory configured offline -> version, persistent offline demo, license extraction, revision inspection and reopened conversation all succeed; inspect runtime provisioning for the absence of download paths and distinguish this from an OS egress firewall test.
- [ ] 5.2 @integration (agent) Prepare the Windows bundle through the memory-owned mise task, then build from local verified inputs without network -> the executable contains the target payload, not the host payload; missing/corrupt input fails the build.
- [ ] 5.3 @regression (agent) Feed duplicate physical ZIP records, inconsistent local/central metadata, links, encryption, extra members and compressed/expanded/final-output overflow fixtures -> validation rejects before executable probing or publication and preserves an existing cache/install.
- [ ] 5.4 @integration (agent) Inspect actual shipping Kuru and embedded Dolt PE imports and build with explicit MSVC target/static CRT settings -> only OS DLLs are required and host proc macros/build scripts still compile normally.

## 6. Native CLI, command and file authority [critical]

- [ ] 6.1 @integration (agent) Exercise CLI config/data defaults with HOME absent, explicit overrides, drive/UNC classification and Unicode/spaced paths -> selected native paths and precedence match the documented commands.
- [ ] 6.2 @integration (agent) Run compiled MCP/auth fixtures and supported command shims, configured cmd /c npx and deliberate single-source cmd forms with empty/quoted/metacharacter arguments, unusable preceding PATH entries and case-varied environment keys -> supported argv/environment and explicit interpreter intent are preserved, device authentication is bounded, and no unrequested shell command executes.
- [ ] 6.3 @integration (agent) Run shell CRUD/cancellation fixtures under stock native PowerShell and file tools against junction/ADS/device/trailing-alias escapes -> authorized operations work with correct output/status and authority violations preserve external content.
- [ ] 6.4 @eval (agent) Replay deterministic agent tool requests for ordinary shell/file work and Windows alias/metacharacter escapes through the actual native tool dispatcher -> authorized work succeeds, unauthorized side effects are absent and the model-facing result preserves the existing authority/error contract without live provider calls.

## 7. Actual Windows terminal behavior [critical]

- [ ] 7.1 @e2e (agent) Run real ConPTY chat, selector, navigation, resize and cancellation scenarios -> rendered state and persistent choices match expected behavior without relying solely on a virtual buffer.
- [ ] 7.2 @regression (agent) Send native focus-loss events and wait for a completed render frame -> no later animation bytes appear while unfocused; focus return resumes appropriate behavior.
- [ ] 7.3 @integration (agent) Exit the TUI normally and exercise partial terminal initialization plus an error after entering terminal mode -> native console modes and owned processes are restored/closed through the shared session guard; Unix restoration assertions still execute on Unix.

## 8. Compiler-free installation and loaded-image update [critical]

- [ ] 8.1 @e2e (agent) Run package-owned PowerShell bootstrap in stock PowerShell 5.1 with actual locally packaged release bytes and an isolated destination -> no checkout/compiler/Bash/Dolt is needed and the resulting executable completes the persistent offline demo.
- [ ] 8.2 @regression (agent) Exercise explicit flags/env precedence, latest-manifest version freezing, checksum failure, malformed archive, unsafe destination, output bounds and real cancellation -> rejected installs preserve exact old bytes and clean owned staging/producers.
- [ ] 8.3 @e2e (agent) Run actual native mise GitHub selection/install/activation against simulated release metadata and genuine candidate ZIP bytes through documented URL replacements, with isolated config/data/cache/credentials and asserted requests -> the exact installed digest/version and persistent offline demo/reopen pass, corruption fails before activation, absent API digests use genuine SHA256SUMS, browser-to-API fallback stays inside the fixture, and no live fallback or user installation satisfies the test; actual published installation remains separately pending in docs/release.md.
- [ ] 8.4 @regression (agent) Update the actual running Windows fixture executable and interrupt each recorded publication/helper boundary -> success publishes verified bytes, handled failure restores old bytes, recovery is deterministic, candidate code is never used as the validator/helper, and owned cleanup completes after old-image exit.
- [ ] 8.5 @integration (agent) Run native source installation through the prepared app mise task into an explicit directory -> the resulting executable contains the correct embedded runtime and persists/reopens the offline demo.
- [ ] 8.6 @regression (agent) Keep an old process alive, rename its loaded image and replace its original pathname with a byte-identical different file -> current-image preparation rejects the stale instance before copying or launching helper code, while a normal instance pins its actual source until the copy finishes.

## 9. Required CI and release integration [critical]

- [ ] 9.1 @integration (agent) Run required package format/lint/test/coverage tasks on native windows-2025 and retain existing Unix jobs -> all actual test counts and measured coverage are recorded, at least 90% workspace coverage remains enforced, and missing Windows prerequisites fail instead of skipping.
- [ ] 9.2 @regression (agent) Exercise release planning/recovery/asset inventory fixtures with five targets -> one prepared version SHA is retained across reruns, invalid/missing Windows artifacts block publication and published assets remain immutable.
- [ ] 9.3 @integration (agent) Validate the release workflow graph and actual native build artifact -> Windows uses the prepared version commit, packaged runtime smoke passes, and Pages remains inside the release workflow after successful publication.
- [ ] 9.4 @manual (agent) Review README, guides and AGENTS against implemented native commands, package ownership and observed evidence -> mise/PowerShell precede source, embedded runtime needs no separate setup, update semantics are accurate and no unobserved hosted result is described as passed.

## Implementation evidence before native acceptance

The candidate is still being integrated. No checkbox above is satisfied by a
host-only compile or an authored Windows fixture. The archived foundation
records retain their separate native evidence at `ca68d7d`; they do not establish
acceptance of these consumer changes.

On macOS arm64, the independent ZIP codec passed nine behavioral tests and its
package coverage gate measured 230/235 lines (97.87%). After checking the actual
pinned Windows Dolt ZIP, its narrowly allowed p7zip POSIX attribute marker was
corrected. The memory package then passed all 65 tests, including actual upstream
Windows ZIP decoding and hashes, live Dolt persistence/recovery and committed-WAL
legacy import. Memory and connector strict host Clippy passed; all 33 connector
host tests passed. The platform package passed 22 host tests and Windows-target
strict Clippy, including the added console and checked file-removal APIs.

The integrated application passed host all-target type checking and all 49
host behavior tests in 138.61 seconds. This includes actual PTY interaction,
preferences, writer leases, A2A shutdown, native Unix self-update and a packaged
offline install/update conversation. Existing test-only subprocess entrypoints
are invoked by their parent scenarios; the separate frame benchmark remains
explicitly ignored. Private-directory fixture setup was corrected so authority
tests reach their intended checks. An unintended new three-member requirement
on the Unix tar reader was removed, preserving its established optional-docs
contract while Windows ZIP retains its exact inventory.

The delivery suite passed 100 host tests, with strict Clippy and ShellCheck.
The existing two-child cold-import test exposed and now covers the corrected
exclusive-create/existing-open behavior of Unix stable locks. Further memory
checks passed 15 provisioning tests and two actual process/lease regressions,
including a demonstrated failure before and success after retaining probe
resources independently of the caller's destroyed Tokio runtime. Memory Clippy
passed after those corrections. Windows-target platform Clippy also passed the
new current-image guard; its stale-image behavior still needs native execution.

The integrated macOS arm64 `mise run check` completed successfully in 449.43
seconds on 2026-09-10. Combined workspace coverage measured 13,791/14,193 lines
(97.17%), above the unchanged 90% gate. Formatting, strict linting, type checking,
documentation validation and ordinary plus instrumented behavioral suites all
passed. The log is `/tmp/kuru-native-windows-full-check.log`; this local result
does not establish Windows consumer acceptance.

A repeated pre-push gate exposed a concurrent Cargo fixture race: another
package could start tests while Cargo replaced the top-level supervisor
executable. The memory-owned prefetch now publishes a private, immutable
supervisor snapshot for ordinary test tasks. A regression removes and replaces
an isolated copy of the actual compiled alias, then starts Dolt and reopens its
committed data through the retained snapshot. That regression, the corrupted
snapshot case and strict memory Clippy passed. Combined coverage explicitly
clears the ordinary snapshot opt-in and uses its instrumented executable.

The corrected full `mise run check` passed in 464.29 seconds, with 13,919/14,353
covered workspace lines (96.98%). The log is
`/tmp/kuru-native-windows-followup-check.log`. This includes the Unicode/spaced
migration fixtures and preserves all existing Unix behavior. Native picker
selection, focus-resume and reader-cleanup checks, modeled Windows tool replay,
drive/UNC classification and pinned DB/WAL assertions are authored but await
Windows execution. Platform Windows-target Clippy passed the added path tests.
The Windows command facade's missing `envs` method was also corrected before
native consumer CI; host cfg checks alone could not have detected that omission.

Native Windows consumer compilation and execution remain pending on CI. A local
cross-target attempt stopped in aws-lc's C compilation because the macOS host
has no Windows SDK; no dependency, TLS configuration or test contract was changed
to turn that limitation into a reported pass. Shipping PE import inspection,
stock PowerShell bootstrap, loaded-image updates, ConPTY and actual mise-backend
installation have authored native checks but no recorded native result yet.

The first native consumer candidate, `939edcc`, ran in
[CI 34527303037](https://github.com/replygirl/kuru/actions/runs/34527303037).
The Windows primitive job passed 48 ordinary tests, then the current-image
fixture exited before its initial acknowledgment. Coverage did not complete.
The full Windows job failed before application tests: Windows checkout CRLF
conversion broke ShellCheck and caused cospec's managed files to appear modified.
Repository text now has an explicit LF checkout policy, and the image fixture
reports the failing stage, OS error and child status while completing owned
cleanup. The guard's behavior is unchanged; neither fix has native acceptance yet.

Additional updater fixtures interrupt the real helper at prepared receipt,
old-image move, candidate move, published receipt and cleanup boundaries, and
separately exercise parent loss after acknowledgment. These tests reconcile
recorded transaction files and exact native identities through the stock
PowerShell recovery path. They remain unexecuted on Windows. Killing the parent
before acknowledgment can leave its private, unrecorded candidate scratch
directory: recovery has no recorded identity authorizing its deletion. These
fixtures do not claim to clean that scratch directory or use pathname-pattern
deletion to conceal it; ordinary completed-update cleanup remains required.

The added native memory fixtures observe creator loss after partial readiness
consumption and during an actual in-flight SQL transaction, normal Dolt close,
forced engine-adapter cleanup after both compiled descendants acknowledge a
console break, concurrent cold provisioning and lifecycle contention through an
interrupted-directory move. They remain unexecuted on Windows. The separate
post-move completion-error regression passed against actual Dolt on macOS
(one test, 2.87 seconds); it exercises real identity reconciliation and staged
recovery, without claiming a process kill at ready-marker publication. Memory
all-target type checking and strict host Clippy passed.

The platform's new partial-frame fixture checks the received prefix, bounded read
cancellation, lifetime EOF, child/output closure and completed Tokio destruction.
Windows-target strict Clippy passed in 4.14 seconds; native execution remains
pending. This compile result also includes the improved image-failure diagnostics.

The same `939edcc` CI run completed both macOS jobs successfully. The arm64 full
check took 629.32 seconds and its source-install/offline-runtime checks passed;
Intel macOS passed 70 memory cases and packaged offline-runtime verification.
Both Linux jobs exposed Cargo's legitimate hard-linked build outputs being
rejected by the packaging source reader. Packaging and local source installation
now retain and verify read-only build snapshots while preserving strict output
and installed-file checks. Regressions exercise a genuinely linked compiled
fixture across all five archive targets, same-size mutation, source-name
replacement and linked destination preservation.

The corrected local full `mise run check` passed in 458.85 seconds on macOS
arm64, measuring 14,078/14,491 covered lines (97.15%). Formatting, strict linting,
type checking, documentation/cospec checks and ordinary plus instrumented suites
passed. The log is `/tmp/kuru-native-portability-corrections-check.log`.
Windows and Linux execution of these corrections remains pending; the current
image change supplies diagnostics and does not claim to repair the guard failure.

The next candidate adds actual creator-interruption fixtures immediately before
and after ready-marker publication in the real initial import path, plus an
observer-EOF cleanup control. A single private, bounded pause leaves public
configuration unchanged and awaits existing database cleanup on ordinary errors.
Two real-Dolt host tests passed in 6.03 seconds, covering release cancellation at
both boundaries, recovery and successful continuation; memory type checking and
strict Clippy also passed. The native process-kill cases remain unexecuted and
are not established by these host controls. Logs are
`/tmp/kuru-ready-marker-{typecheck,tests,lint}.log`.

The next native run, [CI 34530648227](https://github.com/replygirl/kuru/actions/runs/34530648227)
at `fffae33`, confirmed LF checkout behavior: cospec reported no managed drift
and ShellCheck no longer rejected carriage returns. All 17 process/IPC cases,
including partial-frame cancellation, passed. The current-image fixture reported
`PermissionDenied` without an OS error: the two native path queries disagree for
the unchanged loaded executable. Additional lossless path diagnostics now observe
both the initial and renamed image. A diagnostic-only fallback lets that fixture
complete the rename experiment, but cannot satisfy its acceptance assertion;
production guard conditions are unchanged and remain unresolved.

The full Windows job next found that opening a directory as configuration fails
before its regular-file diagnostic. A preflight classification now gives the same
actionable error on Windows while retaining opened-handle validation. Core tests
and strict Clippy passed locally. CI continues independent tasks after failures,
and ordinary package tests collect failures across test binaries. A disposable
fixture verified that pinned mise completed independent work while retaining the
failing task's exit status 37; thresholds and required checks are unchanged.

The same `fffae33` run passed Linux ARM packaging, native Dolt verification and
the packaged offline-runtime smoke. This confirms the hard-linked build-input
correction on that target; it does not establish Windows application acceptance.

The completed `fffae33` run also passed both macOS targets. The arm64 full check
took 695.98 seconds and passed source-install and installed-runtime checks;
Intel macOS passed 71 memory cases and packaged offline-runtime verification.
Ubuntu x64 instead exhausted the hosted runner's disk: GitHub's annotation
records `System.IO.IOException: No space left on device` while writing its worker
log. No application assertion was established from that aborted job.

The Ubuntu full-check job now removes only its unused preinstalled Android and
Swift SDK roots before setup/cache restoration, reporting disk usage before and
after. The paths are defined by the official runner image's
[Android installer](https://github.com/actions/runner-images/blob/main/images/ubuntu/scripts/build/install-android-sdk.sh)
and [Swift installer](https://github.com/actions/runner-images/blob/main/images/ubuntu/scripts/build/install-swift.sh).
Cargo outputs, test coverage, the runner runtime and cache settings are retained;
the reclaimed capacity and native result remain to be measured on CI.

The local full `mise run check` for the new configuration, image diagnostics and
marker fixtures passed in 688.55 seconds, with 14,305/14,731 covered workspace
lines (97.11%). Ordinary and instrumented tests, formatting, strict lint, type
checking, docs and cospec checks passed. The subsequent CI-only storage adjustment
is checked separately through repository tooling. The full log is
`/tmp/kuru-native-diagnostics-check.log`; no new Windows execution is claimed.

The next native candidate, `ab5fd52ea6bcc3e5eff25cd9b6e0afcf51acf17b`, ran in
[CI 34533962889](https://github.com/replygirl/kuru/actions/runs/34533962889).
Its Windows primitive job passed 49 cases in both ordinary and instrumented
runs: 10 unit, 18 filesystem, four command/console and 17 process/IPC cases.
The current-image fixture failed in both runs, so there is no completed native
coverage measurement. The initial trace identified an opened short-name path
(`RUNNER~1`) versus a normalized long-name path (`runneradmin`). The subsequent
rename experiment stopped with sharing violation OS32: the fixture itself still
held a delete-denying source handle. This did not establish whether the loaded
mapping name follows a rename.

The local image candidate now requests documented `FILE_NAME_OPENED` together
with `VOLUME_NAME_NT`, preserving exact UTF-16 comparison and diagnostics. The
fixture retains the parent and a movable same-identity source view, verifies
the rejected first move preserved identity/bytes and an absent destination,
then requires the actual moved identity/bytes and original-name absence before
creating the byte-identical replacement. Stale-image rejection remains mandatory;
a diagnostic fallback can never satisfy the test. Windows-target strict Clippy
passed, but the opened-name comparison still requires the next native run to
prove rename freshness. No case folding or undocumented image API was added.

The full Windows job exposed three further independent blockers. Seven connector
unit cases failed before subprocess startup because a Cargo output on `D:` could
not be hard-linked into the temporary directory on `C:` (OS17). Both exact shell
stderr tests also observed PowerShell's first-use progress serialized as CLIXML.
The fixture now retains one bounded verified executable snapshot on its own
volume before creating aliases; the shell prelude disables only progress display
before any cmdlet. Native regressions retain exact Unicode/status assertions,
literal CLIXML-looking stderr, warnings and errors. All 33 connector host tests
and strict host Clippy passed; these corrections have no native result yet.

Windows delivery compilation failed because the updater's spawned handoff future
captured a shared `NativeChild` borrow across an await; its owned pipe state is
intentionally not `Sync`. The local correction constructs the listener's owned
peer-identity accept future before the handoff block, without adding `Sync` or
changing process ownership. Strict host delivery Clippy passed in 2.57 seconds;
native consumer compilation remains pending. The coordinated connector and
delivery Windows-target Clippy attempts both stopped in `aws-lc-sys` 0.45.0
before reaching consumer Rust checking because this macOS host has no Windows
SDK `windows.h`. Both exited 101; no SDK, TLS or source-policy change was made
to hide that limitation (`/tmp/kuru-windows-consumer-crosscheck.md`).

Memory preparation was blocked by that delivery compilation failure and exposed
a separate path error: `target/kuru-bundles` retained an ordinary forward slash
when the platform added an extended-path prefix, producing OS123. Memory now
joins native components separately and tests missing-directory, missing-file and
verified default-input identities. The shared platform boundary converts only
ordinary drive-path separator code units before adding its internal prefix;
explicit verbatim paths with forward slashes still reject. Native identity/byte
checks and raw unpaired-surrogate preservation are authored, while reparse and
invalid-component guards remain intact. Six focused memory build-input tests and
strict host Clippy passed; platform Windows-target Clippy passed in 0.85 seconds.
Neither is native Windows behavior evidence. The full Windows application,
bootstrap, updater and ConPTY suites remain blocked behind these compilation
and setup failures.

The same `ab5fd52` run passed the macOS arm64 full check (591.06 seconds), source
installation and installed offline-runtime acceptance. Linux ARM passed native
packaging, 72 memory cases and packaged offline-runtime acceptance (30.71 seconds).
Intel macOS passed native packaging, 73 memory cases and packaged offline-runtime
acceptance (103.60 seconds). Ubuntu x64 passed its full check in 34 minutes
8 seconds and moved to source installation. Its SDK cleanup step succeeded;
exact reclaimed bytes, source/offline acceptance and the completed job result
are still pending. Logs are retained under `/tmp/kuru-windows-ab5fd52-*`; no
acceptance checkbox is satisfied merely by these partial native or local
cross-target results.

The integrated local `mise run check` for these corrections passed in 460.00
seconds on macOS arm64. Combined workspace coverage measured 14,304/14,731
lines (97.1014%), with the unchanged 90% gate. Formatting, strict lint, type
checking, docs/cospec validation and ordinary plus instrumented behavior passed.
The log is `/tmp/kuru-native-corrections-check.log`. The corrected Windows
image, command, bundle-path and updater behavior still requires native CI;
this local pass does not satisfy those acceptance rows.

## Native results and delivery corrections after a268ad9

GitHub CI run `34538076381` at exact commit
`a268ad9c30783cb300274ca9f2ccecabd860d90f` completed with all four Unix jobs and
Windows primitives passing; the full Windows job and required aggregate failed.
Windows primitives passed 52 ordinary and 52 instrumented cases, including real
loaded-image move/replacement identities, ordinary separator conversion and raw
UTF-16 preservation. Native platform coverage was 2,590/2,839 lines (91.2293%).
Ubuntu workspace coverage was 14,293/14,720 lines (97.0992%); its full check,
source installation and installed offline-runtime probe passed. The native
Windows connector suite passed 29 unit and five integration tests. Evidence is
in `/tmp/kuru-windows-a268ad9-results.md` and the corresponding native job logs.

Full Windows then exposed missing WRITE_DAC access on generic temporary archive
handles and a stock PowerShell startup failure on configured launches. Bundle
preparation now uses the existing checked private Stage/Directory constructor.
The command boundary avoids introducing verbatim executable/cwd syntax when a
valid ordinary UTF-16 spelling resolves to exactly the same canonical target;
long, ambiguous and special spellings retain their original form. A new native
regression compares direct and configured PowerShell launches with identical
minimal environments and requires actual Framework initialization and a script
marker. Another protects distinct trailing-dot targets.

Bootstrap fixtures now require cause-specific rejection or the actual archive
verification marker. Its preexisting cancellation test did reach that marker;
it was not an early-startup false pass. The corrupt-helper fixture now substitutes
a checked object while retaining and restoring the sealed original identity,
instead of attempting to write a correctly read-only helper. The docs fixture
keeps its exact rejection assertion using native path separators.

Scoped macOS checks passed three bundle transport/cancellation/lock units, nine
bundle preparation integration cases, 13 docs validation cases and strict host
delivery Clippy. Platform Windows-target strict Clippy passed. These corrections
still require actual Windows bundle sealing, PowerShell, installation/updating,
memory and ConPTY execution; none of those acceptance rows is checked by local
cross-compilation. See `/tmp/kuru-native-delivery-a268-corrections.md` and
`/tmp/kuru-docs-native-path-tests.log`. The concurrent `parallel-quality-gates`
change changes scheduling, retaining the native behavior and coverage gates.

## Native server fixture correction after d66780f

Run `34542165125` at d66780f passed all seven independent static categories and
Windows primitive coverage: 54 instrumented cases and 2,602/2,865 lines
(90.8202%). Both new PowerShell startup and distinct-verbatim-target regressions
passed natively. Windows bundle preparation also succeeded. Full application
compilation then found E0308 in the A2A server fixture: two borrowed paths were
passed to `NativeSpawnSpec::new`, which requires owned `PathBuf` arguments.
The test now converts both explicitly; an audit of all 23 constructor calls
found no other ownership mismatch. The focused macOS A2A server test and scoped
strict Clippy passed. Windows application test execution is still pending;
compilation stopped before that suite ran. Evidence:
`/tmp/kuru-native-server-path-fix.md` and `/tmp/kuru-windows-d66780f-full.log`.
