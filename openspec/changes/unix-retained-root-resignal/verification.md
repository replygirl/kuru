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

## 3. Existing caller contracts [critical]

- [ ] 3.1 @integration (agent) macOS hook, shell, RPC/MCP and coverage cleanup suites -> final connector library 304/304 passed; earlier delivery coverage tests 35/35 passed on PR207 baseline; final PR216 deadline integration and delivery rerun still pending
- [x] 3.2 @integration (agent) runtime cancelled-dream hook-descendant acceptance -> final cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants passed 1/1 on macOS using package-owned offline bundled Dolt preparation and command-local nofile4096; initial soft256 attempt failed before hooks on memory-template EMFILE and is recorded separately
- [~] 3.3 @integration (agent) hosted Linux native platform and caller acceptance -> defer: this host is macOS; no hosted Linux job was dispatched from this local branch, and local results do not establish Linux acceptance
- [x] 3.4 @unit (agent) pending-helper hook expiry and retained-shell reaped-phase rounds -> injected Pending and ExpiredPending expiry states exact-reaped real exited hook roots, physical child rows disappeared and timeout remained unconfirmed; retained reaped round returned after one injected pending observation, with actual absence confirmed only in the next round

## 4. Cost and repository checks

- [x] 4.1 @benchmark (agent) fixed before/after successful hook/shell batches and concurrent batch -> actual same-binary before/after final macOS selector receipt: hook sequential32 median/p95 12.687/13.597 to 25.790/38.575 ms, concurrent8 13.779/26.021 to 28.838/53.462; shell sequential32 8.190/10.156 to 20.989/22.532, concurrent8 19.178/24.054 to 47.359/78.367; before zero ps calls, after exactly32/8 per route. About13 ms sequential median added cost remains; existing caller cadence is unchanged. One-job exited-root and zero-job running-root controls passed; all temporary cost seams removed. Detailed raw local receipts are ignored tmp/roadmap, not public docs
- [ ] 4.2 @integration (agent) scoped format, lint, typecheck, docs build/check and strict Cospec validation -> observed checks pass without changing pins or unrelated files
