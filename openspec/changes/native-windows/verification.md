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
