## 1. Native process ownership and supervisor handoff [critical]

- [ ] 1.1 @integration (agent) Run compiled native fixture children and grandchildren while killing the owner at process creation/startup handshakes -> owned commands enter their Job atomically, no descendant escapes, and unrelated processes survive.
- [ ] 1.2 @integration (agent) Launch concurrent children with distinct inherited handles and a root that exits before its grandchild -> private handles do not leak and quiescence waits for the actual final process and output closure.
- [ ] 1.3 @regression (agent) Kill a real supervisor caller before configuration, during acknowledgment and during accepted SQL work -> startup is bounded, accepted work follows reconciliation, a second writer cannot overlap, and the supervisor releases the lease only after Dolt is quiescent.
- [ ] 1.4 @integration (agent) Exercise graceful shutdown and forced escalation on the headless Windows runner -> Dolt closes normally where cooperative and its Job reaches verified zero active processes before any protected resource is released.

## 2. Private IPC cancellation and identity [critical]

- [ ] 2.1 @integration (agent) Stall named-pipe connection, partial frame reads and blocked writes, then cancel and shut down Tokio -> every operation finishes within its bound without stranded blocking readers or leaked handles.
- [ ] 2.2 @integration (agent) Present an existing pipe name, wrong connected process and concurrent unrelated child -> first-instance/ACL/peer checks reject impostors before private configuration transfer and lifetime EOF is not held open by leaked handles.

## 3. Filesystem privacy, identity and durable transitions [critical]

- [ ] 3.1 @integration (agent) Create native permissive/null/unsafe inherited ACL fixtures, junctions, other reparse points and hardlinks at protected roots/leaves -> private operations reject them without changing foreign data; legitimate Dolt descendants with inherited owner-only grants remain usable.
- [ ] 3.2 @integration (agent) Attempt pathname substitution under a held lock and concurrent cache publication -> full handle identity prevents overlapping ownership; exactly one valid publication is retained.
- [ ] 3.3 @regression (agent) Interrupt file/directory activation at recorded rename/marker boundaries on native NTFS, including an uncertain post-move result -> stable lifecycle locking and identity/receipt reconciliation preserve a valid state, including failure preservation and no-clobber behavior without assuming every error means no move occurred.

## 4. Real Dolt behavior on Windows [critical]

- [ ] 4.1 @integration (agent) Run real storage/runtime suites with the embedded full Windows Dolt -> private histories, stable IDs, preferences, sessions, transaction rollback and close/reopen match the existing contract.
- [ ] 4.2 @regression (agent) Migrate SQLite fixtures with committed WAL content, Unicode/spaced paths and source replacement attempts -> backup is lossless, held identities prevent substitution, source is preserved, and interrupted activation recovers correctly.
- [ ] 4.3 @regression (agent) Cancel or conflict a complete dream candidate, then exercise promotion, lost acknowledgment reconciliation and compensating undo after later chats/preferences -> no partial dream reaches live state and later writes survive undo.

## 5. Embedded bundle and ZIP validation [critical]

- [ ] 5.1 @e2e (agent) Run only the actual packaged Windows executable with an empty cache, no external Dolt/compiler on PATH and runtime network unavailable -> version, persistent offline demo, license extraction, revision inspection and reopened conversation all succeed.
- [ ] 5.2 @integration (agent) Prepare the Windows bundle through the memory-owned mise task, then build from local verified inputs without network -> the executable contains the target payload, not the host payload; missing/corrupt input fails the build.
- [ ] 5.3 @regression (agent) Feed duplicate physical ZIP records, inconsistent local/central metadata, links, encryption, extra members and compressed/expanded/final-output overflow fixtures -> validation rejects before executable probing or publication and preserves an existing cache/install.
- [ ] 5.4 @integration (agent) Inspect actual shipping Kuru and embedded Dolt PE imports and build with explicit MSVC target/static CRT settings -> only OS DLLs are required and host proc macros/build scripts still compile normally.

## 6. Native CLI, command and file authority [critical]

- [ ] 6.1 @integration (agent) Exercise CLI config/data defaults with HOME absent, explicit overrides, drive/UNC classification and Unicode/spaced paths -> selected native paths and precedence match the documented commands.
- [ ] 6.2 @integration (agent) Run compiled MCP/auth fixtures and supported command shims with empty/quoted/metacharacter arguments plus case-varied environment keys -> argv and environment are preserved and no unrequested shell command executes.
- [ ] 6.3 @integration (agent) Run shell CRUD/cancellation fixtures under stock native PowerShell and file tools against junction/ADS/device/trailing-alias escapes -> authorized operations work with correct output/status and authority violations preserve external content.
- [ ] 6.4 @eval (agent) Replay deterministic agent tool requests for ordinary shell/file work and Windows alias/metacharacter escapes through the actual native tool dispatcher -> authorized work succeeds, unauthorized side effects are absent and the model-facing result preserves the existing authority/error contract without live provider calls.

## 7. Actual Windows terminal behavior [critical]

- [ ] 7.1 @e2e (agent) Run real ConPTY chat, selector, navigation, resize and cancellation scenarios -> rendered state and persistent choices match expected behavior without relying solely on a virtual buffer.
- [ ] 7.2 @regression (agent) Send native focus-loss events and wait for a completed render frame -> no later animation bytes appear while unfocused; focus return resumes appropriate behavior.
- [ ] 7.3 @integration (agent) Exit the TUI normally and through startup/interaction errors -> native console modes and owned processes are restored/closed; Unix restoration assertions still execute on Unix.

## 8. Compiler-free installation and loaded-image update [critical]

- [ ] 8.1 @e2e (agent) Run package-owned PowerShell bootstrap in stock PowerShell 5.1 with actual locally packaged release bytes and an isolated destination -> no checkout/compiler/Bash/Dolt is needed and the resulting executable completes the persistent offline demo.
- [ ] 8.2 @regression (agent) Exercise explicit flags/env precedence, latest-manifest version freezing, checksum failure, malformed archive, unsafe destination, output bounds and real cancellation -> rejected installs preserve exact old bytes and clean owned staging/producers.
- [ ] 8.3 @e2e (agent) Install and activate the native ZIP with mise's GitHub backend for an actual published exact version -> `mise exec -- kuru --version` resolves the correct executable using isolated mise config/data and no user installation changes.
- [ ] 8.4 @regression (agent) Update the actual running Windows fixture executable and interrupt each recorded publication/helper boundary -> success publishes verified bytes, handled failure restores old bytes, recovery is deterministic, candidate code is never used as the validator/helper, and owned cleanup completes after old-image exit.
- [ ] 8.5 @integration (agent) Run native source installation through the prepared app mise task into an explicit directory -> the resulting executable contains the correct embedded runtime and persists/reopens the offline demo.

## 9. Required CI and release integration [critical]

- [ ] 9.1 @integration (agent) Run required package format/lint/test/coverage tasks on native windows-2025 and retain existing Unix jobs -> all actual test counts and measured coverage are recorded, at least 90% workspace coverage remains enforced, and missing Windows prerequisites fail instead of skipping.
- [ ] 9.2 @regression (agent) Exercise release planning/recovery/asset inventory fixtures with five targets -> one prepared version SHA is retained across reruns, invalid/missing Windows artifacts block publication and published assets remain immutable.
- [ ] 9.3 @integration (agent) Validate the release workflow graph and actual native build artifact -> Windows uses the prepared version commit, packaged runtime smoke passes, and Pages remains inside the release workflow after successful publication.
- [ ] 9.4 @manual (agent) Review README, guides and AGENTS against implemented native commands, package ownership and observed evidence -> mise/PowerShell precede source, embedded runtime needs no separate setup, update semantics are accurate and no unobserved hosted result is described as passed.
