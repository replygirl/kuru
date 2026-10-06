# Verification

## 1. Long public paging remains usable and private [critical]

- [x] 1.1 @runtime (agent) exercise the existing 1,025-turn actual memory fixture with 32-record pages and causal other-session writer progress -> record observed work/progress, exact coverage, cursor/revision rejection and the evidenced storage decision; no arbitrary millisecond gate
- [x] 1.2 @integration (agent) verify bounded adjacent pages, exact record omissions, fork prefix, pending projection and stale/forged continuations -> no unrelated or private rows and no full-session retained view
- [x] 1.3 @e2e (agent) drive long public history outside the initial viewport at 80/120 columns -> completed frames show fetched older content and responsive input with bounded retained state

## 2. Reading and literal search preserve identity [critical]

- [x] 2.1 @integration (agent) exercise stable item/wrapped-row anchors across stream replacement, card expansion, long-item layout and resize -> same identified item/valid row while follow-tail remains distinct
- [x] 2.2 @e2e (agent) search beyond the visible page with resize at 80/120, and separately exercise a Reading anchor during held streaming -> matches are public and literal; scan/cancellation states and source guards are honest, without claiming the two interactions were exercised simultaneously
- [x] 2.3 @integration (agent) cancel/change query, switch session and clear while a page/search reply is pending -> stale results cannot publish; draft and saved anchor remain literal

## 3. Input and owned terminal state remain correct [critical]

- [x] 3.1 @e2e (agent) exercise transcript navigation and modal paste/search priority alongside existing approval, picker, recall, paste/history and card controls -> default mouse is paired by the guard; modal ownership and line-local composer Home/End remain unchanged
- [x] 3.2 @integration (agent) capture terminal writes for supported/unsupported title mechanisms and injected session controls -> actual title stack restoration or no title mutation, paired mouse/focus/paste cleanup and no escape injection
- [x] 3.3 @e2e (agent) observe unfocused successful completion, replay/cancel/failure and terminal exit/refusal -> fixed content-free completion signal only for a current successful new turn, with restored terminal state

## 4. Repository acceptance and delivery boundaries

- [x] 4.1 @regression (agent) run relevant owning granular format, host/Windows lint, typecheck, docs and structural fixture guards -> actual terminal results recorded against stable source
- [x] 4.2 @integration (agent) independently review production seams and run strict/apply with returned context -> record actual clear gate and scoped local acceptance before real archive
- [~] 4.3 @regression (agent) final archive, conventional commit/hooks and hosted native/90% coverage -> defer: ordered after local implementation/acceptance; actual archive and final commit will be reported when performed, hosted checks belong to later normal delivery
- [~] 4.4 @runtime (human) paid live-provider smoke and final phase release -> defer: selected during final full-phase acceptance under the existing policy; no live account or release action belongs to this local feature slice

## Observed local outcomes

### Public storage and bounded projection

The existing real-memory 1,025-turn fixture passed at 32 records per page (82062, 1/1). The final changed-reader selection (26909, exit 0, two exact cases) retained full coverage, exact totals, fork-prefix/forged/stale rejection and added a causal writer proof. A task-local one-shot holds the reader after the selected committed cut is established; a second session's actual Admit checkpoint receipt completes while that reader remains held. Releasing the reader returns the captured coherent page, and its continuation becomes stale after the writer's revision. No cursor hash selects a pool. The reader no longer holds the mutable writer mutex through traversal or decodes unreturned bodies. Full metadata traversal still repeats per page: whole-history search remains quadratic in metadata query work, not a claimed linear implementation or benchmark.

Five deterministic cases passed in 85685: bounded page eviction/exact omissions, live/persisted slots and local dedup, every within-page literal match before older pages, literal draft/recall priority, and title-stack sanitization/capability skip. The final 98585 selection passed six deterministic/guard cases: Unicode/card-height anchor restoration, modal paste priority, causal query/session/view/clear read cancellation with rejected late results, retained C1 notices plus explicit accepted omissions, actual-layout composer paging, and the established store-fixture closing guard. Those accepted cases were not broadly repeated.

### Completed-frame terminal behavior

The long-history PTY passed 1/1 in 98585 at 120 and 80 columns: 96 seeded public turns, a 70,000-line Unicode item, older literal answer/user matches, deep Reading anchor restoration after search and resize, literal composer input, unsafe escape exclusion and owned terminal cleanup. This fixture does not claim simultaneous searching while a provider is held.

The separate existing controlled-stream PTY passed 1/1 in 2695 (6.79 s): a saved Reading anchor remains visible through held streaming and practical 80/120-column resize; 40-column preview/privacy assertions remain; returning from the zero-transcript-area viewport preserves the anchor, explicit forward navigation reaches follow-tail, FocusLost precedes provider release, and actual successful completion emits exactly one fixed BEL with private text absent. A final deterministic anchor selection passed 1/1 in 10820, including forward navigation from a supported zero-height transcript layout without resize/redraw implicitly switching to follow-tail. Variable card-height and Unicode row mapping are deterministic render evidence; cross-feature combinations not present in these PTYs are source-reviewed rather than claimed as additional observed interactions.

The single existing completion-order case passed 1/1 in 18095 (1.41 s; owning task exit 0), preserving current successful completion and adding negative notification assertions for reused turns, commands, typed cancellation and stale generations. No real mouse-event PTY or full supported-target native matrix is claimed by these local selections.

### Corrections and static outcomes

67253 stopped before tests at an incorrect Cargo/libtest argument boundary. 85685 exposed saved-anchor clamping/restoration; the final source preserves identified reading rows and only one projection/width-bound active viewport byte/row/code resume point. 98585's controlled-stream addition initially asserted visible transcript content at 40×18 where the transcript had zero rows; the assertion was moved to practical sizes while retaining preview/privacy checks. The subsequent 62303 task was interrupted (130) after exposing the real zero-height forward-tail edge; production navigation now uses a virtual minimum one-row tail boundary, and the fixture checks its existing deadline before each frame read. 2695's remaining deterministic failure assumed an unsupported 1×1 layout; 10820 uses the supported 40×7 layout. No deadline was enlarged or behavior assertion weakened.

Final TUI host lint (6445), Windows-target lint (2794), all-target/all-feature typecheck (84080), memory host lint (20997), memory Windows-target lint (14341), memory all-target/all-feature typecheck (82601), root formatting (4211), managed drift (54552), and full owning docs check/build/content (53689) exited 0. Initial docs formatting failure was corrected by the owning formatter. Initial TUI lints identified a large private wake enum; boxing only its PublicRead result preserved ordering and payload without a behavioral change. Root and Architecture independently reviewed the reader, tagged cancellation, bounded render resume, terminal ownership and privacy seams clear. Final strict validation and actual apply exited 0 clear with no blockers, and all six returned contexts were reread fully. All ten implementation/local-acceptance tasks are complete. Actual archive/final commit remain pending at this record update.

Hosted native CI, 90% coverage, paid provider smoke and final release have not run for this slice. Deterministic HTTP/provider fixtures are local acceptance, not paid live-model evidence. Archive, final commit/hooks and later delivery outcomes will be recorded only after they occur.
