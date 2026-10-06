# Verification

## 1. Candidate cancellation [critical]

- [x] 1.1 @regression (agent) cancel a real managed candidate read after its frame was sent, release its response, then read and abandon the same candidate -> baseline35558 terminal101, 0 passed/1 failed with the exact candidate-attachment-loss error (2.11s); corrected50727 terminal0, 1 passed/0 failed (1.85s), original head readable, exact ref abandoned and main unchanged; actual owner retirement succeeded on both outcomes.
- [x] 1.2 @e2e (agent) existing headless held-output/broken-pipe/exit-dream completed-retry fixture -> owning39950 terminal0, exact unchanged terminal fixture1 passed/0 failed (13.17s): real held exit dream SIGINT returns130 after cleanup, held/broken output and setup cancellation remain intact, completed retry uses exact durable public output without inference replay.
- [x] 1.3 @integration (agent) relevant owning closing guard -> same39950 terminal0, existing app guard1 passed/0 failed (0.03s); all other target cases were filtered out. New managed fixture uses existing FixtureDeadline.serve plus guarded root.release to retire its owner on error before releasing its root.

## 2. Required checks

- [x] 2.1 @integration (agent) affected host/Windows lint and all-target typecheck -> memory host93132, Windows79674 and all-target/all-feature typecheck28760 all terminal0.
- [x] 2.2 @integration (agent) docs, format, strict validation and managed drift -> docs10053, format84285, managed drift and actual strict/apply gates each terminal0; five returned contexts read. Immutable old archives are untouched. Final closure repeats only strict/apply after evidence status updates; actual archive and normal hooked commit follow.

## Observed scope

Exact merged-main4a598765 Ubuntu coverage partition2 job112167721546 failed the existing headless fixture with `turn cancelled; memory cleanup failed: candidate attachment was lost`. Four scoped source reviews (author, root, Preflight and delivery owner) agree the causal cancelled-read path; an already reached provider does not prove that all parallel participants finished context reads. The finite task preserves the existing per-frame/reply deadlines, not a new total35s budget. Mutating calls, exact outcome recovery, candidate no-reconnect guard and private disclosure policy remain unchanged.

Initial owning88402 stopped on a Cargo/libtest argument separator error before tests; corrected55237 stopped on the fixture's digest formatting compile error before tests. Neither is behavior evidence. Canonical scope formatting was corrected before the red35558 regression. Source then changed only in the candidate nonmutating path; cargo formatting was mechanical after green50727.

Docs10053 terminal0 includes site build, local links and anchors. Format84285 and managed drift each terminal0. Separate docs/format setup tasks briefly collided installing existing Git hooks; normal setup subsequently succeeded, both checks exited0, and the final commit will run the normal hooks. Hosted Ubuntu instrumented acceptance, native Windows execution and workspace90% coverage are unrun locally and remain required remote CI evidence. No paid provider calls, dependencies, tool pins, schema or wire changes.
