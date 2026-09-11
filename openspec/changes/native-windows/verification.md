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

- [ ] 9.1 @integration (agent) Run required package test/coverage tasks on native windows-2025 and retain existing Unix jobs, with format/lint/typecheck running independently on Ubuntu per `parallel-quality-gates` -> all actual test counts and measured coverage are recorded, at least 90% workspace coverage remains enforced, and missing Windows prerequisites fail instead of skipping. Linux static analysis does not establish checking of Windows conditional branches; the native suite must compile and exercise them.
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

## Final acceptance boundary review

Source-to-test maps are recorded in `/tmp/kuru-native-memory-acceptance-map.md`,
`/tmp/kuru-native-delivery-acceptance-map.md` and
`/tmp/kuru-native-cli-terminal-acceptance-map.md`. They distinguish assertions
present in source from actual hosted execution. This review identified five
specific boundaries that a green existing suite alone would not establish.

The added memory fixture terminates a retained real creator before it sends any
supervisor configuration. It requires the supervisor's actual UnexpectedEof
completion and output closure, with no database or lease state. The engine now
returns a crate-private shutdown outcome and appends it to the existing private
log after cleanup; public lifecycle behavior and resource ordering are unchanged.
The real-Dolt close fixture requires Graceful, and the stubborn compiled adapter
requires Forced. The existing real-Dolt lifecycle scenario passed on macOS with
the new Graceful assertion (one test, 3.07 seconds), alongside type checking and
strict Clippy. The new Windows creator/stop assertions remain unexecuted there.
Evidence: `/tmp/kuru-memory-lifecycle-final-evidence.md`.

Stock PowerShell bootstrap regressions now exercise the actual compressed-file
cap, aggregate expansion cap and declared member/output bounds with correctly
checksummed physical ZIP records. A real loaded-image update fixture retains a
candidate handle that denies DELETE sharing, proves OS32, then resumes the
helper. It requires automatic restoration of the old object's identity/bytes
before any recovery command. After the handle closes, recorded recovery must
finish pending cleanup. Candidate execution and unrelated-file changes remain
forbidden. Host type checking and strict Clippy passed; these Windows bodies
still require native compilation and execution. Evidence:
`/tmp/kuru-native-delivery-limits-rollback.md`.

The existing native source-install step now disables Cargo network access and
missing-bundle downloads after dependency preparation. Windows additionally
runs the memory-owned `bundle:verify-native-build` task against actual Cargo,
using isolated missing and same-size corrupted mirrors, then a valid offline
build with the shipping target/profile. Specific build-script failure causes,
original archive hashes and installed executable hashes are required; no nested
Cargo test or extra static-analysis job was added. Source review, formatting,
actionlint and cospec validation passed. Actual Windows invocation is pending;
no local PowerShell interpreter was available for native execution.

## Complete native run after 627edee

CI run `34545817609` at `627edee5d10ff99f303d0e024c729e3124550b3a`
completed with all seven static categories, all four Unix native jobs and the
Windows primitives passing. The full Windows instrumented suite completed with
363 passing cases, 21 failures and one intentionally ignored profiling case;
ten targets failed. Its required aggregate failed, and subsequent Windows
coverage publication, source installation, offline Cargo-input rejection,
installed shipping-runtime and PE-import checks did not run. Compilation took
7 minutes 13 seconds; the complete coverage/preparation step took 2,681.87
seconds. Completion with failures is not evidence of a hung job.

The actual Windows passes include all 54 platform cases, 29 connector units and
five native connector cases, 31 memory units, ConPTY cancellation and independent
console restoration, pre-configuration creator loss, both marker-kill scenarios,
Graceful real-Dolt shutdown and Forced stubborn-adapter shutdown. Primitive
coverage was 2,602/2,865 lines (90.8202%). Full Windows coverage and consumer
acceptance remain open despite these component passes.

Ubuntu workspace coverage was 14,299/14,725 lines (97.1070%) and macOS arm64 was
14,314/14,736 (97.1363%); both source installation and installed offline-runtime
checks passed. Linux arm64 and Intel macOS passed their real memory and packaged
offline-runtime checks. Logs and LCOV artifacts are retained under
`/tmp/kuru-windows-627edee-*`, including the full Windows log and failure index.

The failures distinguish production defects from fixture defects: memory
bootstrap/query failures, PowerShell/.NET working-directory spelling, updater
handoff and recovery, mise command lookup, physical ConPTY cursor coordinates,
and archive/ACL diagnostic and sharing assertions. Corrections are being
validated separately. No native acceptance row is satisfied by their source
changes or by a local host test; a new native run must demonstrate the resulting
behavior.

The common memory correction was demonstrated against actual Dolt on macOS:
the unchanged six-second SQL query failed after 5.001108125 seconds with the
old five-second listener timer, then passed in 7.66 seconds including startup
and close. Dolt's result-iterator timer now derives from the greater of the
configured startup limit and the existing 30-second query budget. Caller
deadlines, write timeout, cancellation and shutdown are unchanged. Six existing
timeout, cancellation, session-teardown and archive controls and strict host
Clippy passed. Static bootstrap-phase diagnostics and bounded marker-fixture
startup-result frames expose any remaining distinct failures. The native schema
rejection/recovery and raw traversal-input regressions still require Windows.
Evidence: `/tmp/kuru-native-memory-627-fixes.md`.

The connector shell now uses the established checked Windows command adapter,
which retains trusted PowerShell selection and permits an ordinary cwd spelling
only after its exact canonical round-trip. The regression performs the real
.NET file write in a spaced Unicode directory and checks its bytes as well as
stdout, stderr and status. Independent source review passed; the new path has
not run natively. Evidence: `/tmp/kuru-windows-shell-review.md`.

ConPTY composer checks now use physical `Screen::rows`, since logical contents
join autowrapped rows. An isolated probe of the old predicate failed and the
corrected predicate passed, including partial, hidden and misplaced cursor
controls. The fixture also gives normal and error teardown the same bounded
console owner, with a native delayed-close regression. This separate cleanup
gap is not established as a cause of the completed Windows 2025 failure.
Formatting passed; actual new ConPTY execution remains pending. Evidence:
`/tmp/kuru-conpty-cleanup-correction.md`.

Delivery recovery preserves the validated receipt's helper launch spelling,
with existing normalized ACL checks, full file identity, checksum and strict
loaded-image validation intact. Its corruption fixture reads the retained
source through a same-identity movable view beneath the pinned cache so its
own handle does not prohibit the intended rename. Stock PowerShell diagnostic
assertions retain the exact required words across output wrapping. The isolated
mise environment now supplies conventional PATHEXT; a real missing-PATHEXT
control precedes successful activation checks. Independent source review
passed after correcting one remaining wrapped-diagnostic assertion.

Application updater handoff is still unresolved. Phase-specific errors and a
continuously drained, bounded helper stderr prefix now distinguish connection,
request, publication and helper-exit outcomes. An observed exit drains buffered
errors to EOF; an unobserved exit preserves the private candidate stage instead
of deleting a possible live source. No updater timeout or identity/hash rule
changed. A native compiled fixture checks output beyond the capture cap and
final error retention. Integrated formatting, 102 delivery host tests and strict
host lint passed; these Windows-only cases require the next native run.
Evidence: `/tmp/kuru-native-delivery-627-fixes.md` and
`/tmp/kuru-delivery-path-review.md`.

## Local cold gate and hosted startup block at 1943c4e

The normal concurrent pre-push passed all eight hooks. Its initially absent
private Dolt cache was populated by the instrumented fixtures, and actual cold
startup, durable memory, installed offline update and runtime checks passed.
Combined coverage was 14,328/14,757 lines (97.092905%); the instrumented suite
took 260.25 seconds and coverage with input preparation took 261.90 seconds.
Cargo artifacts were reused, so this is not a clean-build performance comparison.
Evidence: `/tmp/kuru-627-corrections-push.log` and
`/tmp/kuru-1943c4e-cold-coverage.lcov`.

The resulting hosted CI `34551251564` and PR-title workflow `34551251314`
failed before runner execution. All 15 CI jobs had zero executed steps and
zero artifacts. GitHub's annotations report that recent account payments failed
or the spending limit must be increased. No corrected Windows case, updater
diagnostic, hosted coverage, source installation or shipping probe ran. Native
acceptance and the unresolved app handoff remain pending; no retries, billing,
Release or Pages actions were performed. Raw annotations and job metadata are
recorded in `/tmp/kuru-windows-1943c4e-results.md` and
`/tmp/kuru-ci-1943c4e-results.md`.

## Hosted execution resumed at 1234096

After the user changed repository visibility, the normal push passed all eight
local hooks and [CI 34552780285](https://github.com/replygirl/kuru/actions/runs/34552780285)
executed actual hosted jobs. This evidence-only commit retains the source from
1943c4e. All seven static jobs, their required aggregate and the PR-title check
passed. The previous account startup refusal did not recur.

All four Unix application jobs passed. Ubuntu measured 14,318/14,746 lines
(97.097518%) and macOS arm64 measured 14,329/14,757 (97.099682%); both passed
source installation and installed cold offline install/update conversations.
Linux arm64 passed 74 memory cases and Intel macOS passed 75, followed by their
actual release-built packaged offline install/update checks. These results use
the shipping executable selected by each native job, without publishing assets.
The separate Windows primitive job passed 54 cases with 2,602/2,865 lines
(90.820244%).

The full Windows application job completed with 381 passing tests, eight
failures and one intentionally ignored timing benchmark; seven test targets
failed. Compilation took 3 minutes 35 seconds and the coverage/preparation task
took 2,537.35 seconds. The final aggregate failed as required. All 62 memory
cases and all ten delivery updater cases passed, including real recovery,
Graceful/Forced teardown, marker interruption and bounded helper diagnostics.
The remaining failures concern CLI shell timeout, full-app installed/source
update, unexpected mise fixture traffic, ConPTY label capitalization, two
bootstrap assertions and runtime cancellation-fixture setup.

The source updater's new diagnostics identify a publication-acknowledgment
timeout with unobserved helper exit and empty stderr. The candidate stage was
retained. This does not establish a hashing bottleneck or resolve the failure.
Windows combined LCOV, source installation, offline Cargo-input controls,
installed shipping-runtime and PE-import checks did not run after the failed
suite. Native consumer acceptance remains open. Exact sizes, timings, logs,
failure locations and LCOV paths are recorded in
`/tmp/kuru-ci-1234096-results.md` and
`/tmp/kuru-windows-1234096-results.md`.

The ConPTY correction changes only the fixture's reopened-screen expectation
from `jungian` to the actual displayed `Jungian`. The retained native screen
fails the old predicate and passes the corrected one. Its completed-frame,
physical-cursor and quiescence requirements are unchanged.

The runtime cancellation failure occurred before provider entry, while the
turn persisted its transcript/input and read private history. The fixture now
synchronizes on actual provider entry under the existing real-dream setup
budget, then retains a separate two-second bound across abort, provider-drop
acknowledgment and reacquisition of the real sole pool permit. A completed
recovery turn remains required. Two focused real-Dolt host tests and strict
runtime Clippy passed; no product deadline or actor behavior changed.

The CLI shell's thirty-second timeout is still unresolved. Bounded captures now
retain received bytes and actual EOF observations across cancellation, name
output-read versus process-wait failures, and report the retained child's state
and cleanup result. Both pipe closes are attempted and awaited. Three focused
host capture/shell tests and strict connector Clippy passed; independent source
review found no blocking defect. Execution and cleanup bounds, native shell
selection and successful output/status semantics are unchanged. These source
corrections and diagnostics require new Windows execution. Evidence:
`/tmp/kuru-conpty-123-correction.md`, `/tmp/kuru-runtime-123-correction.md` and
`/tmp/kuru-shell-123-correction.md`.

Delivery's bootstrap fixtures now decode the actual stock PowerShell CLIXML
Error records and remove the deliberately corrupt helper through the existing
same-identity movable view. Warning/progress-only and empty Error envelopes
cannot satisfy rejection assertions. A real pinned host mise probe reproduced
the unexpected version-notification request; its exact URL now has a local
fixture route, while all other unexpected traffic remains rejected. These are
fixture corrections; native ACL, image, checksum and recovery rules are unchanged.

The trusted updater now emits finite static phase names with elapsed time to
its already drained stderr. The Windows command facade preserves bounded output,
EOF observations and the original failure across failed process-tree waits and
cleanup. A new native fixture requires an exited root's final error to survive
its live descendant, then proves owned teardown releases the descendant's real
file lease. The packaged app fixture labels each installation, conversation and
update stage. Existing publication, capture and cleanup deadlines are unchanged;
the actual ACK and shell timeout causes remain unresolved pending native output.

All 105 host delivery tests, strict delivery Clippy and formatting passed. The
first sandboxed test attempt was refused permission to bind existing loopback
listeners; the same task passed with approved fixture binding. Independent
review caught and resolved the CLIXML no-error false-positive path and found no
remaining issue. These host results do not typecheck or execute Windows branches.
Evidence: `/tmp/kuru-native-delivery-123-fixes.md`,
`/tmp/kuru-mise-123-version-routing.log` and
`/tmp/kuru-delivery-123-review.md`.

The normal push of `a552102f5e868c5e3e55d18a1c058835c97f6833` passed all eight
concurrent local checks. Combined coverage was 14,392/14,820 lines (97.112011%);
the suite took 263.68 seconds and coverage with preparation took 265.70 seconds.
The hosted static jobs and PR-title check passed in
[CI 34557801363](https://github.com/replygirl/kuru/actions/runs/34557801363).
Native jobs were still running when the next fixture issue was found; no native
acceptance is inferred. Local evidence: `/tmp/kuru-a552102-push.log` and
`/tmp/kuru-a552102-coverage.lcov`.

Further integration review found an effect missed by the first source review:
interrupted-helper and post-ACK fixtures still required empty stderr despite the
new static phase records. The loaded-image assertion also accepted unrelated
lines while locating its expected phases. Their shared test-only parser now
requires bounded, complete UTF-8 records with known phases and canonical,
nondecreasing elapsed values; extra text remains an error. Actual failure and
recovery assertions, identity checks, marker synchronization and deadlines stay
in place. The original push completed before it could be stopped; no process or
hosted workflow was manually cancelled.

Three focused host parser controls, strict delivery Clippy and formatting passed.
Independent review checked every emitted phase and affected interruption/error
path; it found no remaining issue. The source comparison is not native execution.
Evidence: `/tmp/kuru-helper-trace-followup.md` and
`/tmp/kuru-helper-trace-followup-review.md`.

## Normal helper profiling destination

While native `037d470` ran, a source audit found that ordinary app update helpers
dropped the runner's `LLVM_PROFILE_FILE` at their second process hop. The observed
delivery fixture forwarded it explicitly, so those passing fixtures did not cover
this difference. The shared updater now forwards only that existing profiling
destination; fixture-only execution-marker authority remains in its callback.
No broader environment, profile naming, deadline or publication rule changed.

A fresh native-windows apply returned clear and its context was read before the
one-file correction. Independent source review, strict host delivery Clippy and
formatting passed. Host conditional compilation does not execute or typecheck
the Windows module; normal native helper execution and profile emission remain
pending. This is a coverage-routing correction, not evidence about the ACK
timeout's cause. Evidence: `/tmp/kuru-update-helper-context-audit.md` and
`/tmp/kuru-update-profile-correction.md`.

## Hosted 037d470: measured full-image verification cost

[CI 34558398251](https://github.com/replygirl/kuru/actions/runs/34558398251)
finished with all static checks, all four Unix application jobs and the separate
Windows primitive gate passing. Ubuntu measured 14,380/14,809 lines (97.103113%)
and macOS arm64 measured 14,392/14,820 (97.112011%); both source-installed shipping
binaries passed cold offline install/update conversations. Linux arm64 passed
74 memory cases and Intel macOS 75, plus their actual packaged offline checks.
Windows primitives passed 54 cases with 2,602/2,865 lines (90.820244%).

Full Windows completed with 396 passed, two failed and one intentionally ignored
frame-cost measurement. All 62 memory, 44 runtime, 12 PowerShell bootstrap,
ten updater and five ConPTY cases passed, as did real native mise acceptance and
the prior CLI shell failure. The new strict trace and native descendant-output
controls passed. Only the packaged self-update and source-update assertions failed.
The required aggregate failed; full Windows LCOV, lock/source-install checks,
offline Cargo-input controls, installed shipping runtime, PE imports and upload
were skipped after the suite. The task took 2,332.38 seconds including 3 minutes
36 seconds of compilation. Exact logs and complete counts are retained in
`/tmp/kuru-windows-037d470-results.md` and `/tmp/kuru-ci-037d470-results.md`.

Both failures now expose helper progress: installation locking completed in
13/36 milliseconds, verification before copying in about 8.6/8.9 seconds,
candidate/rollback preparation by about 21.5 seconds, and verified publication
reached ACK send at 28.241/28.354 seconds. The parent had already reached its
unchanged ten-second ACK deadline, so the helper exited with OS232 on its closed
pipe. This locates the failure after actual publication rather than at startup
or lock acquisition. These phases include reads and allocation, not just hashing.

Source audit counted 15 full-image digests before ACK. Ordinary held file handles
share writes and do not make content immutable, so removing later verifications
would weaken mutation detection. Conservative overlap reductions alone would
still leave most observed work. The planned narrow SHA-2 code-generation
experiment preserves those independent checks and every timer. Source rationale
and limitations: `/tmp/kuru-update-cost-invariants.md` and
`/tmp/kuru-sha2-profile-audit.md`.

- [x] 10.1 @integration (agent) Compare the existing and narrowly optimized SHA-2 dependency on identical bytes with unchanged consumer/instrumentation settings -> matched macOS arm64 artifacts hashed the same 64 MiB in median 270.508083 ms versus 27.439500 ms (15 observations each, three alternating pairs). All 45 measured digests, including supplementary runs, matched independent OpenSSL. No Windows or end-to-end update result is inferred.
- [ ] 10.2 @integration (agent) Run unchanged native full-app updates, full native coverage and shipping checks -> verified publication is acknowledged within existing budgets, normal helper profiles retain the runner destination, and actual Windows acceptance passes before closing the change.

The experiment used the pinned Rust 1.98.1 compiler and a frozen disposable Rust
harness, with no additional manifest, workspace or dependency. The primary before
and after SHA-2 artifacts had identical version, features and dependency
fingerprints. The consumer retained opt-level zero, coverage instrumentation and
line-table debug information. The dependency remained uninstrumented in both
arms, matching cargo-llvm-cov 0.9.1's existing workspace selection. Cargo artifact
JSON confirmed SHA-2 0.11.0 at opt-level three and delivery at zero, with debug
assertions and overflow checks enabled; the separate SHA-2 0.10.9 was unchanged.
Digest computation alone improved 9.858346 times; the measurement excludes IO,
allocation and actual Windows execution. Evidence and exact data:
`/tmp/kuru-sha2-cost-report.md` and `/tmp/kuru-sha2-build-artifacts.jsonl`.

Independent source reviews found no profile, instrumentation, integrity or
environment-isolation regression. The packaged update fixture now gives only the
update command a unique profile prefix in the runner's existing merge directory,
then requires nonempty files from both distinct processes within ten seconds.
This directly observes the ordinary helper's second process hop; the files remain
available to the normal LCOV merge. Explicit shipping-binary acceptance remains
separate because that binary is not instrumented. Actual native execution of this
observation is pending. Reviews: `/tmp/kuru-sha2-change-review.md`,
`/tmp/kuru-sha2-wrapper-review.md` and `/tmp/kuru-native-windows-closure-review.md`.

Strict host app Clippy passed, including the complete portable profile-observation
module; only its execution is conditional on native Windows instrumentation.
Attempted Windows-target app Clippy on macOS stopped in AWS-LC's C compilation
because this host lacks Windows SDK headers, after the initially missing locked
crates were fetched. This is not a Windows compile pass; the actual native CI
remains required. Evidence: `/tmp/kuru-sha2-app-clippy.log` and
`/tmp/kuru-sha2-windows-clippy-network.log`.

## Public Windows source entrypoint

A manual docs/source review found that `scripts/install.ps1 -Source` invoked
mise before disabling automatic task-tool installation and setup hooks. Its
inner app-owned script applied those settings too late for first task activation;
the native CI source smoke supplied them globally and could mask this path.
The planned correction scopes both settings before the first invocation and
restores the caller's environment even when the task fails. Existing native
PowerShell/compiled-command fixtures will observe the actual entrypoint with
absent and opposing settings. This is separate from the still-running c52fa62
native update experiment; no passing execution is inferred for the correction.

- [x] 11.1 @regression (agent) Invoke the real source entrypoint under native PowerShell with absent and opposing mise settings and controlled success/failure of its compiled command fixture -> both native entrypoint cases passed on 87271ce, including all four environment/status combinations and invalid-option rejection; actual source compilation/installation passed separately in the same run.

The same manual review found one small command-table error: `format:fix` also
formats the documentation app. The development guide now states its complete
write scope. No other incorrect command was found across README, canonical
AGENTS, install/auth/config guides and release/CI ownership. Default Codex remains
explicitly external until its separately gated bundle is implemented. Review:
`/tmp/kuru-native-docs-c52fa62-review.md`. Final native and entrypoint acceptance
remain pending; this documentation comparison does not substitute for them.

The entrypoint correction and two native test cases are implemented. The tests
exercise four success/failure and absent/opposing environment combinations, plus
invalid source/release options before any mise call, using the real PowerShell
entrypoint and a compiled invocation recorder. They do not claim an actual
compiler installation or source build. Independent source review found no
blocking issue; format, tooling and strict cospec checks passed in 1.33 seconds.
Actual native execution remains pending on a subsequent candidate; c52fa62 does
not contain this correction. Evidence: `/tmp/kuru-source-entry-apply.json`,
`/tmp/kuru-source-entrypoint-independent-review.md` and
`/tmp/kuru-source-entrypoint-static.log`.

## Terminal redraw dispatch after c52fa62

The completed native run passed both full-application updates but failed the
focus-loss quiet-output assertion. Its log retained no appended bytes, so the
failure does not identify their cause. Source inspection independently found
that the Windows backend forwards key releases and the real loop marks every
event dirty before the editor discards releases. This permits unchanged frames
after the last character is visible. Correct this shared dispatch boundary and
retain native byte/cursor diagnostics without changing its 450 ms quiet interval
or 700 ms animation observations. Actual native acceptance is still required.

- [x] 12.1 @regression (agent) Exercise the real dispatch boundary with focus loss, typed press/repeat and trailing releases, including command keys -> the focused host regression passed against the dispatcher used by the real loop, proving editing/command/release behavior and overdue focus/cadence handling; independent source review confirms other pending redraw causes are retained.
- [x] 12.2 @integration (agent) Run the actual native ConPTY scenario with bounded escaped late-output diagnostics -> the entire combined ConPTY case passed on 87271ce, including unchanged focus-loss quiescence, resumed animation, later selectors/resize/persistence and reopened reduced-motion assertions; all five native terminal cases passed.

The focused host regression passed in 1.11 seconds (22.07 seconds including
prepared memory and compilation). Formatting and strict cospec validation passed.
Both native quiet-output assertions now retain bounded escaped pre/post bytes,
cursor visibility/position and screen diagnostics without altering the 450 ms
observation or filtering any output. Independent review found no material defect;
native execution and the separate frame-prefix synchronization risk remain open.
Evidence: `/tmp/kuru-focus-dispatch-test.log`, `/tmp/kuru-focus-dispatch-apply.json`
and `/tmp/kuru-focus-dispatch-final-review.md`.

## Hosted c52fa62 result

[CI 34562838599](https://github.com/replygirl/kuru/actions/runs/34562838599) passed
all static jobs, four Unix application jobs and Windows primitives. The actual
merge checkout was `c78539c8e2234ddc434c3a56da67a6e0a14f5e3c`; its source tree
`36409b13d99ea1c708b57edc7df35efa7b195ee0` matches branch head `c52fa62`.
Full Windows recorded 397 passed, one failed and one intentionally ignored case
in 1549.48 seconds including compilation. Both normal full-app updates passed
within unchanged deadlines and with all 15 verification digests retained. The
packaged case observed profiles from the ordinary app/helper pair. Successful
stderr was captured, so no exact successful ACK timestamp is available.

Only the ConPTY focus-output assertion failed; its later selector/persistence
assertions were not reached. The final gate failed, and full Windows LCOV,
source installation, offline Cargo, shipping runtime and PE-import checks were
skipped. The separately passing Windows primitive gate measured 2602/2865 lines
(90.820244%); Ubuntu measured 14381/14809 (97.109866%) and macOS arm64
14392/14820 (97.112011%). All four Unix jobs passed actual packaged offline
install/update checks. The source-entrypoint correction was not in this tree.
No release or Pages deployment occurred. Exact results and limitations:
`/tmp/kuru-windows-c52fa62-results.md` and `/tmp/kuru-ci-c52fa62-results.md`.

## Stock PowerShell task activation after 87271ce

The native instrumented suite, coverage gate and actual source installation
passed in CI 34565545466. The following offline Cargo-input check failed before
its first hash: stock Windows PowerShell reported `Get-FileHash` unavailable.
The job was launched by PowerShell 7 through mise. Microsoft's documented
[cross-edition module-path behavior](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath?view=powershell-7.6#starting-windows-powershell-from-powershell-7)
matches this launch chain and error: an intermediate executable preserves the
PowerShell 7 module path that a directly launched 5.1 child would have removed.
The log establishes the missing command; it did not print the inherited module
path, so that causal link remains an inference pending the corrected native run.

Remove only inherited `PSModulePath` in each package-owned mise task that
explicitly selects stock Windows PowerShell: source installation, offline
Cargo-input verification and shipping PE inspection. The child can reconstruct
its own standard module paths. Preserve configured user shell/MCP environments,
all archive/hash/identity assertions, target settings and runtime dependencies.
No custom hash implementation or separately installed shell is needed.
Upload the existing coverage artifact immediately after the successful coverage
and lock checks, so a later shipping-check failure cannot discard that evidence.
All installation/PE steps and the required aggregate still must pass.

Independent review found the same concrete inheritance chain in the built-in
Windows shell tool: it deliberately launches stock PowerShell while forwarding
the complete Kuru environment. Apply the same single-key removal at that owned
launch site, using native case-insensitive key comparison. Keep generic
configured commands, explicit MCP/provider environments and unrelated inherited
values intact. A native child-process regression must supply an incompatible
module path without mutating the test runner environment, then execute a real
stock cmdlet through Kuru and verify its result plus unrelated environment
preservation. This source exposure is not an observed failure of the passing
87271ce shell cases.

- [ ] 13.1 @integration (agent) Run the stock PowerShell mise tasks through the native hosted PowerShell 7 launcher after scoped module-path removal -> real source installation, missing/corrupt/valid offline Cargo controls, installed offline runtime and Kuru/Dolt PE inspection pass; checksums and exact old-byte preservation remain required.
- [ ] 13.2 @regression (agent) Invoke Kuru's built-in Windows shell with an incompatible inherited module path in an isolated child -> actual stock cmdlet output and unrelated inherited values survive, while a direct unsanitized control demonstrates the unavailable command; configured command/MCP environment behavior stays unchanged.

The completed log is `/tmp/kuru-windows-87271ce-full.log`; the failure appears
at 05:52:59Z after successful release compilation and `kuru 0.1.0` execution.
The final CI aggregate failed. Shipping-runtime/PE verification and coverage
artifact upload were skipped; this is not full native acceptance.

The completed Windows report records 401 passed, zero failed and one intentionally
ignored timing benchmark across 59 result rows. All 62 memory, 44 runtime,
12 stock PowerShell bootstrap and ten updater cases passed. Both ordinary
full-app updates passed with unchanged timeouts; the packaged test observed
nonempty profiles from the distinct normal app/helper pair. The suite took
1520.91 seconds and preparation/suite graph 1634.90 seconds. The successful
coverage command enforced `--fail-under-lines 90`, but its exact full Windows
percentage is unavailable because the later failure skipped artifact upload.
The separate primitive gate passed 54 cases at 2602/2865 lines (90.820244%).

All static jobs, PR Title and all four Unix application jobs passed. Ubuntu
measured 14462/14889 lines (97.132111%) and macOS arm64 14472/14900 (97.127517%);
both source installations and shipping offline install/update checks passed.
Linux arm64 and Intel macOS passed 74/75 real memory cases and their actual
release-built packaged checks. The actual checkout was merge commit
`1acc447e6cddc920c453540a7113cfc79590aedf`, whose complete tree
`143c2a180e94b028e33bab82e386605b05355bdc` matches branch 87271ce.
Exact counts, artifact sizes, timings and limitations are retained in
`/tmp/kuru-windows-87271ce-results.md` and `/tmp/kuru-ci-87271ce-results.md`.
No release or Pages publication occurred.

The scoped module-path corrections and native regression are implemented.
The regression uses a real unsanitized stock PowerShell failure control, then
requires Kuru to return a file's independently calculated SHA-256 and an unrelated
Unicode environment value without changing file bytes or identity. Formatting,
strict host connector Clippy, repository/workflow validation and strict cospec
validation passed. The first sandboxed formatter attempt could not access the
macOS system-configuration service; the approved normal check passed. Host
checks do not typecheck or execute the Windows-only regression. Native
verification 13.1 and 13.2 remain pending. Evidence:
`/tmp/kuru-powershell-shell-correction.md`,
`/tmp/kuru-powershell-correction-clippy.log`,
`/tmp/kuru-powershell-task-tooling.log` and
`/tmp/kuru-powershell-correction-final-validate.log`.

Independent review found no remaining material issue after replacing the native
test's unsupported whole-digest uppercase formatting with byte-wise formatting
compatible with the pinned SHA-2 output type. The actual hash and assertions
are unchanged. Review: `/tmp/kuru-powershell-correction-independent-review.md`.

## Hosted 662ce90 and discriminating shell observations

[CI 34569009783](https://github.com/replygirl/kuru/actions/runs/34569009783)
finished with 400 full-Windows cases passed, two failed and one intentionally
ignored benchmark. The actual merge checkout `e48b2af1e323bfdb12f2810cae7e012856ff3770`
has tree `4945d9b468ec0c37e5dccc79343fe113b5973f71`, identical to branch 662ce90.
All five ConPTY cases, both full-app updates with ordinary app/helper profiles,
both source-entrypoint regressions, actual mise installation, 62 memory,
44 runtime, 12 bootstrap and ten updater cases passed. Windows primitives
passed 54 cases at 2602/2865 lines (90.820244%). All static and four Unix jobs
passed; Ubuntu measured 14462/14889 lines (97.132111%) and macOS arm64
14473/14900 (97.134228%). All four Unix packaged offline checks passed.

The ordinary CLI shell again timed out at the unchanged 30-second deadline:
neither capture contained bytes or EOF, whole-tree status was unavailable, and
owned-tree termination succeeded. This does not distinguish a running root
from a live descendant or establish whether the controlled source executed.
The new module-path regression failed earlier in its own control: stock
PowerShell successfully found the cmdlet despite the empty search directory.
Its positive Kuru assertions were not reached. Full Windows coverage generation,
source installation, offline Cargo, shipping-runtime and PE checks were skipped;
the final aggregate failed. Exact evidence: `/tmp/kuru-windows-662ce90-results.md`
and `/tmp/kuru-ci-662ce90-results.md`.

Correct the module fixture with a discoverable manifest advertising the actual
`Get-FileHash` command but requiring PowerShell 7. No fake cmdlet implementation
or separately installed shell is involved. Microsoft's published engine source
explicitly retains a system-directory fallback for 5.1 command discovery; an
empty directory therefore cannot demonstrate incompatible module shadowing.
Require the real autoload/import failure in the direct unsanitized control and
retain the positive independent hash, Unicode sentinel, file identity and bytes.
The source-supported fixture still requires actual native execution.
Investigation: `/tmp/kuru-powershell-control-662-investigation.md`.

The shell timeout has no demonstrated repair yet. On its existing error path,
query the already-owned root through a wait/query-only duplicate with a zero
timeout before the unchanged tree cleanup. Report root running/exited/query
error independently of whole-tree state, without reopening a numeric PID or
printing environment/arguments. In the existing real CLI fixture, place tiny
private file markers around its original Console.Write, without replacing it,
retrying it or changing its budget. Read only bounded fixture-owned marker
contents after the child returns. Exact successful stdout, stderr and status
remain required; markers distinguish observed source entry/completion but do
not prove pipe delivery or identify an unobserved startup cause. Investigation:
`/tmp/kuru-shell-662-investigation.md`.

- [ ] 14.1 @regression (agent) Run the corrected native incompatible-manifest control and actual Kuru shell -> direct stock autoload fails for the named command/module; Kuru returns the independent SHA-256 and unrelated sentinel with unchanged file bytes/identity and no extra output.
- [ ] 14.2 @integration (agent) Run the ordinary real CLI shell with private source-entry/completion markers and root-state diagnostics -> success requires exact output/status plus actual markers; any failure retains bounded independent observations without retries, extended deadlines or altered ownership. A passing intermittent case alone is not a causal repair claim.

The corrected manifest, bounded source markers and retained-root observation are
implemented. The focused host CLI case passed with exact parsed output/status
assertions in 1.41 seconds (14.70 seconds including package-owned preparation
and compilation). Formatting and strict cospec validation passed. Independent
review found no blocking source issue and explicitly retains the scheduling
limits of marker instrumentation; actual native typing/execution is pending.
Evidence: `/tmp/kuru-shell-observation-cli-test.log`,
`/tmp/kuru-shell-observation-format.log`,
`/tmp/kuru-shell-observation-final-validate.log` and
`/tmp/kuru-shell-observation-independent-review.md`.

## Hosted 563d321 result and native follow-up

[CI 34572200638](https://github.com/replygirl/kuru/actions/runs/34572200638)
finished with 400 full-Windows cases passed, two failed and one intentionally
ignored benchmark. Its actual merge checkout
`a453bf7e3b65f9fcf4d483a655a776774b7d7784` has tree
`10559f0b22cb65f13b33e94a36800ea43a46a55d`, identical to branch 563d321.
The ordinary CLI shell passed with both source markers and exact output/status;
this does not establish a causal repair of its earlier intermittent timeout.
All five ConPTY cases, source-entrypoint regressions, actual mise installation,
12 bootstrap cases and ten updater cases passed. Windows primitives passed
54 cases at 2602/2865 lines (90.820244%). Full Windows LCOV generation/upload,
source installation, offline Cargo controls, installed shipping acceptance and
PE inspection were skipped after the instrumented suite failed.

The incompatible-module control did reach the intended stock PowerShell autoload
failure: its diagnostic names `Get-FileHash`, `Microsoft.PowerShell.Utility` and
`CouldNotAutoloadMatchingModule`. Its wrapper was
`ParentContainsErrorRecordException`, so the additional assertion for
`CommandNotFoundException` rejected the expected failure before Kuru ran.
Correct that overly specific assertion by requiring the named command, module
and fully qualified error ID, plus the existing nonzero status and empty stdout.
Keep the real manifest and every positive Kuru hash, environment and identity
assertion. Native execution of the positive path remains required by 14.1.

The packaged full-application update passed its publication, normal app/helper
profile, identity and digest assertions, then failed the updated executable's
first cold conversation. Dolt directory activation reported uncertain
publication with access denied (OS error 5), followed by file not found
(OS error 2). The log does not identify a causal operation beyond that boundary;
investigate retained handles and publication before proposing a repair. Do not
add retries, weaken identity/durability, or increase existing deadlines.

All static jobs and macOS arm64, Linux arm64 and Intel macOS application jobs
passed, including actual shipping offline installation/update. macOS arm64
measured 14473/14900 lines (97.134228%). Ubuntu failed before any tests on an
upstream GitHub HTTP 500 while preparing the Windows Dolt fixture; the separate
`bundle-download-recovery` change addresses that bounded build-input behavior.
The aggregate failed; neither full native acceptance nor archive is claimed.
Evidence: `/tmp/kuru-windows-563d321-full.log`,
`/tmp/kuru-windows-563d321-results.md` and `/tmp/kuru-ci-563d321-results.md`.

Source review found no normal-path Kuru descendant handle retained at activation.
It did identify two observation gaps: native move and post-move reopen errors
look identical, and the inner temporary stage is removed on activation failure
before the test retains its outer fixture. Add static operation labels preserving
the original OS error/phase, checked name/identity observations on reconciliation
failure, and narrow retention of verified stages on activation error. No claim
is made that those gaps caused the initial access denial. Review:
`/tmp/kuru-dolt-activation-563-investigation.md`.

- [ ] 15.1 @regression (agent) Hold a real descendant file open during native engine activation -> the actual move fails with its operation and original OS error retained; checked source identity, executable/license bytes, absent destination and the owned stable lock are preserved, and the verified private stage survives error return. Closing only the known fixture blocker permits a subsequent explicitly requested checked move of that same identity; product code does not retry.
- [ ] 15.2 @regression (agent) Exercise actual post-move completion-error reconciliation and rejected activation on the host -> successful reconciliation retains exact identity; unresolved activation preserves the verified stage and both original/reconciliation diagnostics, while extraction/probe failure cleanup remains unchanged.
- [ ] 15.3 @integration (agent) Run unchanged actual Windows cold-cache, packaged install/update and shipping acceptance -> each complete path passes; any recurrence records the precise failed operation and checked names without changed deadlines, retries or weakened guarantees. A pass does not establish the cause of the earlier OS5.

The operation labels, original typed-error preservation, checked name observations
and activation-only stage retention are implemented. The host retained-stage
control passed in 0.05 seconds; four real memory move/recovery controls passed
in 3.54 seconds, including actual Dolt completion reconciliation. Six platform
move controls passed, including a real invalid native move and post-move error.
Strict memory Clippy and Windows-target platform Clippy passed; the latter is
type checking, not Windows execution. Rust formatting and independent review
passed. The PowerShell wrapper assertion correction separately passed independent
review. Full hook coverage and native controls remain pending.
Evidence: `/tmp/kuru-activation-563-{retention-test,move-tests,platform-tests}.log`,
`/tmp/kuru-activation-563-{memory-lint,platform-windows-lint,format}.log`,
`/tmp/kuru-activation-563-independent-review.md` and
`/tmp/kuru-powershell-563-control-review.md`.

## Hosted 6fb0a6b result and PE path follow-up

[CI 34576347214](https://github.com/replygirl/kuru/actions/runs/34576347214)
finished with 412 full-Windows tests passed, zero failed and one intentionally
ignored benchmark, at 17928/19006 lines (94.328107%). The actual merge checkout
`5d7bb559a693f32563d3ff29260667a5de7cb482` has tree
`123c838b0b56dddda52634d5b32b80396a3285e5`, identical to branch 6fb0a6b.
The actual incompatible-module negative control and subsequent Kuru hash and
environment assertions passed, as did both source markers in the ordinary shell
case. The real held-descendant activation control passed with preserved verified
stage, identity, original error and stable lock. These controlled successes do
not identify the causes of the earlier intermittent timeout or unexplained OS5.

All five ConPTY cases, normal full-application update and distinct app/helper
profiles, actual native mise fixture, 12 bootstrap and ten updater cases passed.
Windows primitives passed 55 cases at 2640/2903 lines (90.940406%). Actual native
source installation and all three offline Cargo controls passed: missing input
rejected with OS2, same-size corruption rejected by its checksum, restored valid
input built offline. The selected source-installed shipping executable measured
53,709,824 bytes and its outer ZIP 44,159,587 bytes. Its actual packaged install
and self-update each persisted chat from empty offline caches using Dolt 2.3.3.
All seven static/quality/title jobs and all four Unix jobs passed; full Linux
and macOS coverage remained above 97%, with actual shipping roundtrips on all
four Unix targets. The local eight push hooks passed at 14662/15097 lines
(97.118633%).

The final native PE inspection passed Kuru's OS-only imports, then failed closed
because no Dolt imports were parsed. `(Resolve-Path ...).Path` forwarded a
`Microsoft.PowerShell.Core\FileSystem::` provider qualifier ahead of the genuine
extended Windows engine path. Use `.ProviderPath` at both native-argument sites;
do not strip extended prefixes, broaden the DLL allowlist or relax empty-output
and native-exit checks. Actual `dumpbin` compatibility with the resulting native
extended form remains unverified. The aggregate failed; native completion and
archive are not claimed. Evidence: `/tmp/kuru-windows-6fb0a6b-results.md`,
`/tmp/kuru-ci-6fb0a6b-results.md` and `/tmp/kuru-pe-path-6fb-investigation.md`.

- [ ] 16.1 @regression (agent) Run the owning inspection script in stock Windows PowerShell against real verified Dolt PE bytes, forwarding ordinary and provider-qualified extended names to real MSVC tools -> both image inventories are nonempty and accepted, the exact owning prefetch call is recorded, file bytes/identities remain unchanged, and genuine non-PE input fails without a success inventory. Only prefetch command routing is substituted; no fake PE parser, native tool or DLL output.
- [ ] 16.2 @integration (agent) Run the unchanged final native shipping task with actual source-installed Kuru and actual memory-owned prefetch -> both actual OS-only ordinary/delay DLL inventories pass, alongside full native coverage, source/offline Cargo/shipping checks and the aggregate. Local parsing or a fixture prefetch is not shipping provenance evidence.

Independent host byte inspection extracted the pinned Windows Dolt member and
verified its complete executable SHA-256 against the manifest. The existing Xcode
LLVM inspector reported `ADVAPI32.dll`, `KERNEL32.dll` and `msvcrt.dll`, with an
empty delay-import directory. This supports the pinned payload's inventory;
it does not exercise PowerShell, native path forwarding or Windows `dumpbin`.
Evidence: `/tmp/kuru-pe-path-dolt-llvm.txt`.

The representation correction and native regression are implemented. The test
uses three spellings of the same retained real PE, an actual non-PE control and
exact prefetch-call observations; identities and streaming hashes are checked
afterward. Its ASCII launcher explicitly selects UTF-8 for fixture path output,
without changing product encoding. The final real shipping task is unchanged.
Rust formatting, strict cospec validation and independent source review passed.
Windows typing and execution remain pending; this host has no Windows SDK for
consumer cross-checking, and compiled-out Windows tests establish no native pass.
Evidence: `/tmp/kuru-pe-path-implementation.md`,
`/tmp/kuru-pe-path-independent-review.md`, `/tmp/kuru-pe-path-format.log` and
`/tmp/kuru-pe-path-final-validate.log`.

## Hosted 243a79d result and inspection-fixture discovery

[CI 34580970004](https://github.com/replygirl/kuru/actions/runs/34580970004)
finished with 412 full-Windows cases passed, one failed and one intentionally
ignored benchmark. Its actual merge checkout
`683e89a46ceba397f30f127ac9e25da03879b87a` has tree
`2c39665e351222b12241da28970183b82cab96c8`, identical to branch 243a79d.
The sole failure was the new PE regression's first ordinary-path case: stock
PowerShell reached the owning script, but real `vswhere` discovery returned a
nonzero exit or no matching `dumpbin` path. The existing error did not distinguish
those outcomes. It does not establish that MSVC was absent from the runner.
Discovery failed before prefetch, inspection, alternate path forms, invalid-PE
rejection or final file-identity assertions. The ProviderPath correction therefore
still has no completed native acceptance.

Both shell cases, actual instrumented full-application install/update with normal
app/helper profiles, all five ConPTY cases, native mise fixture, 12 bootstrap
cases, ten updater cases and retained-stage controls passed. The full suite's
failure skipped full LCOV generation/upload and all later native source,
offline-Cargo, selected shipping and final Kuru/Dolt PE checks. Windows primitives
passed 55 cases at 2640/2903 lines (90.940406%). All static categories and four
Unix jobs passed, including their actual selected shipping roundtrips. Ubuntu
measured 14652/15086 lines (97.123161%); macOS arm64 measured 14663/15097
(97.125257%). The aggregate failed; no native completion or archive is claimed.
Evidence: `/tmp/kuru-windows-243a79d-results.md`, its completed full log and
`/tmp/kuru-ci-243a79d-results.md`.

Keep the fixture's private user directories and controlled PATH while preserving
the machine-level installation discovery context required by the actual MSVC
tools. Add the real discovery exit status and match count to failure diagnostics.
This fixture adjustment must pass the same unmodified path, non-PE, prefetch and
identity assertions in 16.1, followed by the separate actual shipping checks in
16.2. No simulated tool, discovery fallback, skip or DLL-policy change can satisfy
those rows; the specific environmental cause remains to be confirmed natively.

The fixture now retains only `ProgramData` and `ALLUSERSPROFILE` in addition to
its existing MSVC prerequisites. Microsoft's installation documentation locates
instance state under ProgramData, and its known-folder contract identifies both
machine-root variables; public vswhere code delegates the actual lookup to Setup
Configuration COM. This supports the correction without proving which lookup
failed in 243a79d. The owning script saves the real exit immediately and caps
failure output at 12 lines and 4096 characters. Source review found no changes to
user homes, PATH, product command environments, PE assertions or deadlines.
Strict validation and actual apply both exited zero before source changes;
formatting and independent review passed. Native rows 16.1 and 16.2 remain open.
Evidence: `/tmp/kuru-vswhere-243-investigation.md`,
`/tmp/kuru-pe-discovery-validate.log`, `/tmp/kuru-pe-discovery-apply.json`,
`/tmp/kuru-pe-discovery-format.log` and
`/tmp/kuru-pe-discovery-independent-review.md`.

## Hosted e7eb136: Windows acceptance passed; Unix fixture follow-up

[CI 34584264723](https://github.com/replygirl/kuru/actions/runs/34584264723)
completed full Windows job 103214706180 successfully at 10:08:08Z on September 11.
It passed 413 instrumented tests, with zero failures and one intentionally
ignored benchmark, plus the separate ordinary shipping case. Full raw LCOV was
17928/19006 lines (94.328107%); the separate 55-case primitive suite measured
2640/2903 (90.940406%). Both completed native logs independently confirmed merge
`96e2e2e36c773bba91f51bc3ac149471c8569eb4`, whose tree
`3feabe66b775a192c8a7b335c563bb3345d0a032` equals candidate
`e7eb136fc774639b0a4fbcd2e06d513533c683b4`.

The actual stock-PowerShell/MSVC regression passed all three path forms,
genuine non-PE rejection, exact prefetch routing, and unchanged byte/identity
assertions. Source installation, all three missing/corrupt/valid offline Cargo
controls, and the selected shipping executable's cold offline direct-install
and self-update conversations passed. The real shipping executable was
53,709,824 bytes and its ZIP 44,159,587 bytes. The final unsubstituted native PE
checks also passed for both Kuru and the package-owned prepared Dolt executable,
with nonempty inventories satisfying the unchanged OS-DLL policy. This is native
acceptance of the two PE corrections; it does not identify the particular COM
lookup that failed earlier, or establish published GitHub Release installation.

All seven static categories and three other Unix jobs passed, including actual
selected offline shipping roundtrips. Ubuntu passed 375 instrumented cases with
three intentional exclusions and measured 14652/15086 lines (97.123161%). Linux
ARM passed 76 ordinary memory cases and Intel macOS passed 77; each additionally
passed its selected shipping case.

macOS ARM failed only
`host_detection_selects_each_supported_archive_and_rejects_unknown_hosts` with
the old fixture's generic 30-second timeout. It passed 375 other cases with
three intentional exclusions, then skipped full LCOV, source installation and
shipping. Aggregate 103225613482 therefore failed at 10:08:17Z. No overall native
completion or archive is claimed from this run. The separate
`bootstrap-timeout-observation` fix retains labeled output/root/EOF evidence,
preserves ordinary leak rejection and observes owned cleanup. Its actual failing
controls and corrected 17-case default-feature pass are recorded in its own
verification. The original hosted stall cause remains unknown, and the next
candidate still requires the complete native graph.

Evidence: `/tmp/kuru-windows-e7eb136-results.md`, its completed native logs and
raw coverage artifacts; `/tmp/kuru-ci-e7eb136-results.md`; and the bootstrap
fix's actual strict/apply/RED/GREEN/review records.
