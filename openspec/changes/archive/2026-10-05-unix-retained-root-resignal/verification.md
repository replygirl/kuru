# Verification

## 1. Retained-root second sweep [critical]

- [x] 1.1 @regression (agent) ready real descendant omitted from the first sweep, second-sweep negative control, restored cleanup -> macOS real_second_sweep_removes_omitted_descendant_before_root_reap passed; suppressed second sweep failed the exact live-descendant assertion, real cleanup restored before assertion; positive sweep cleared the descendant before exact root reap
- [x] 1.2 @integration (agent) native finite fork pressure -> macOS finite_native_fork_pressure_settles_before_reap passed eight trials with four workers and 24 finite forks each; this supplements the deterministic regression and does not prove atomic containment

## 2. Authority and helper ownership [critical]

- [x] 2.1 @unit (agent) phases, fresh observation errors, EINTR, permission errors, zombie rows and failed listings -> final macOS platform suite passed 48 unit tests; fresh-error/disarm syscall counts, uncached readiness, zombies, root exclusion and post-reap guards passed; no false ready or expired/reaped/disarmed destructive attempt
- [x] 2.2 @integration (agent) spawn lock held beyond caller deadline and delayed worker admission -> causal held-lock timer/deadline control passed; delayed/cancelled nonexistent-helper admission returned TimedOut/Interrupted without spawn; absolute stalled-helper test returned TimedOut and confirmed helper reap
- [x] 2.3 @unit (agent) cancelled/resumed polling -> retained pending job was not duplicated, expiry produced ExpiredPending, resumed late data was discarded after exact root reap, and no late product-tree syscall occurred
- [x] 2.4 @integration (agent) argument/environment stand-in under inherited legacy mode, native live/zombie/root-only equivalence, and exit-zero stderr failure -> macOS legacy-mode child confirmed exact -g/group/-x arguments, forced unix2003 and LC_ALL=C; live tree and retained root-only zombie matched rich listing; exit-zero stderr failed membership while rich diagnostics retained compatibility; Linux -A branch is source-inspected, not a native Linux pass
- [x] 2.5 @unit (agent) injected ECHILD/unexpected helper observation and first/second reader startup failure -> ECHILD/IO controls disarmed and performed neither Kill nor Wait afterward; EINTR retained wait ownership without Kill; both causal partial-reader startup controls confirmed native helper ECHILD after exact reap and returned the injected error
- [x] 2.6 @unit (agent) completion before wait, cancelled/resumed receiver, deadline/completion tie, worker panic/disconnection and wake before thread exit -> four new macOS controls passed in affected Unix owner suite 26/26; completed work wakes without taking the result, cancelled receiver/job survives, causal publication-to-exit gap stays Pending without reap/signals, expiry retains pending cleanup, and panic/disconnection returns Unobserved rather than readiness. The explicit already-published wake versus expired-limit control also passed its final focused rerun 1/1

## 3. Existing caller contracts [critical]

- [x] 3.1 @integration (agent) macOS hook, shell, RPC/MCP and coverage cleanup suites -> final wakeup connector library 304/304 and delivery coverage 39/39 passed on PR216-integrated source; hook/shell/RPC primary errors, retained cancellation/output cleanup, normal shard deadline and zero-wait controls passed
- [x] 3.2 @integration (agent) runtime cancelled-dream hook-descendant acceptance -> final wakeup cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants passed 1/1 on macOS using package-owned offline bundled Dolt preparation and command-local nofile 4096; initial soft 256 attempt failed before hooks on memory-template EMFILE and is recorded separately
- [~] 3.3 @integration (agent) hosted Linux native platform/caller acceptance and native instrumented PR CI -> defer: this host is macOS and this branch is unpublished; hosted Linux/Windows and combined instrumented coverage remain unrun. Root must obtain fresh native PR CI acceptance before merge, including instrumented Linux/macOS/Windows x64 and Windows ARM behavioral checks; neither local results nor accepted main CI establish this branch's hosted acceptance
- [x] 3.4 @unit (agent) pending-helper hook expiry and retained-shell reaped-phase rounds -> injected Pending and ExpiredPending expiry states exact-reaped real exited hook roots, physical child rows disappeared and timeout remained unconfirmed; retained reaped round returned after one injected pending observation, with actual absence confirmed only in the next round
- [x] 3.5 @regression (agent) causal pending-helper final expiry through normal and interrupted coverage supervision -> real exited roots with causally gated injected pending-helper state stayed unreaped across cancelled/nonfinal zero waits; normal and interrupted production supervision returned unconfirmed, final disposal reaped both exact roots (physical ECHILD), and read-only continuations remained pending until explicit fixture release. Negative control omitting final reap failed the exact physical unreaped-root assertion in both modes and restored exact cleanup before assertion; fixed control passed in final 39/39 suite

## 4. Cost and repository checks

- [x] 4.1 @benchmark (agent) fixed before/after successful hook/shell batches and concurrent batch -> three actual same-binary sequential 32/concurrent 8 receipts with original/reversed/route-interleaved ordering recorded all median/p95 and helper counts below: sequential added medians hook 1.665–2.242 ms, shell 1.721–2.051 ms; concurrent added medians hook 12.246–14.498 ms, shell 10.108–12.114 ms. Before zero ps, after exactly 32/8 per route. All probes/counters/switches removed; snapshot cadence unchanged. Small concurrent batches and baseline outliers do not establish a stable p95 or zero regression. The correctness/latency tradeoff is accepted for delivery with these measurements retained; they cover normal hook/shell paths, not only a test-suite delay. No additional speculative optimization is included. Raw logs are ignored tmp/roadmap

All table values are milliseconds, median/p95. Sequential batches contain 32 samples; concurrent batches contain 8.

| Order / route | Before | After | Delta |
| --- | --- | --- | --- |
| Original: hook sequential | 13.022/13.384 | 14.687/15.111 | 1.665/1.727 |
| Original: hook concurrent | 13.512/13.929 | 26.753/40.966 | 13.241/27.037 |
| Original: shell sequential | 6.062/6.849 | 7.875/8.412 | 1.813/1.563 |
| Original: shell concurrent | 13.125/17.256 | 23.233/33.225 | 10.108/15.969 |
| Reversed: hook sequential | 12.765/13.386 | 14.644/15.613 | 1.879/2.227 |
| Reversed: hook concurrent | 14.803/15.047 | 27.049/39.496 | 12.246/24.449 |
| Reversed: shell sequential | 6.437/10.526 | 8.488/9.713 | 2.051/-0.813 |
| Reversed: shell concurrent | 11.462/14.636 | 23.576/29.316 | 12.114/14.680 |
| Interleaved: hook sequential | 12.181/60.057 | 14.423/16.916 | 2.242/-43.141 |
| Interleaved: hook concurrent | 14.578/15.000 | 29.076/39.991 | 14.498/24.991 |
| Interleaved: shell sequential | 10.630/21.415 | 12.351/16.977 | 1.721/-4.438 |
| Interleaved: shell concurrent | 26.708/35.188 | 37.139/60.100 | 10.431/24.912 |
- [x] 4.2 @integration (agent) scoped format, lint, typecheck, docs build/check and strict Cospec validation -> all three affected package lints/typechecks passed; final format:code, docs formatting/lint/build/public links and anchors passed. Actual strict validation and managed checks pass without dependency, pin, workflow or unrelated-source changes

## 5. Current-main integration and closeout

- [x] 5.1 @equivalence (agent) clean rebase onto exact accepted main -> rebased the two reviewed commits from 4e5ae8c onto 6beb5fb2c980dd6b7b8b744ea677475c7635bb50 without conflicts; new integrated head 8c7180e1a9af71ecd3996e5818ddaeb3e7f95666. Range-diff reports both commits unchanged; all eleven source/documentation files and their composed patch are byte-identical to reviewed 14ce4660. Source patch SHA256 559936fc9a6282ceac4d1a6a811bede2634c3c8eb684a65b8df2fca0ce008bc2. Main's 170 changed paths intersect none of this change's 18 paths; no behavioral suite is repeated solely for the nonoverlapping integration
- [x] 5.2 @integration (agent) scoped static/documentation validation on integrated source -> all nine platform/connectors/delivery host lint, typecheck and Windows-target lint tasks passed exit 0 on the 6beb-integrated source; format:code and cospec:managed:check passed, and docs:check explicitly ran app formatting/lint, documentation build and public links/anchors successfully. Owning offline bundle preparation passed for checksum-verified host and Windows inputs, without a host override on Windows lint. Actual initial, revised and final archive-readiness strict/apply gates exited 0, with no strict warnings and clear apply context; final apply reported all_done, 11/11 tasks complete. No new local behavioral suite or benchmark was started for this nonoverlapping integration
- [x] 5.3 @integration (agent) actual normal Cospec archive and durable spec merge -> archive exited 0: Archived: unix-retained-root-resignal (fix) → openspec/changes/archive/2026-10-05-unix-retained-root-resignal/; Specs: +0 ~1 -0 →0 applied and verified. Active directory is absent and archive is present. Only Owned process tree lifecycle changed, its merged text matches the authored delta exactly and all other requirement bytes are unchanged. No force, skipped specs or manual archive move was used
- [x] 5.4 @integration (agent) actual post-archive strict and managed checks -> mise run cospec -- validate --all --strict exited 0, reported 0 changes/43 specs and 0 errors/0 warnings; informational long-requirement notices remain visible in the log. cospec:managed:check exited 0 with no drift. Historical archived records were not selected or modified

Sections 1–4 retain the actual prior macOS behavioral/cost acceptance of reviewed
14ce4660, including its stated limits and negative controls. Section 5 records
current-main composition and refreshed static/documentation acceptance. Byte
equivalence supports carrying the reviewed local evidence forward; it is not a
newly executed behavioral pass or hosted acceptance.

Native PR acceptance remains mandatory before merge. Actual archive is recorded
above; the final normal-hook commit remains mandatory before publication handoff,
with its actual result supplied in that handoff. The accepted correctness/latency
tradeoff does not establish a hosted test pass or a no-regression claim.
