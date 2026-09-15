## 1. Proven no-move activation recovery [critical]

- [ ] 1.1 @regression (agent) use the actual Windows checked move with a held candidate descendant, observe the first access-denied no-move result, release only that blocker, and continue through the production activation path -> before the fix activation fails after the first denied move; after the fix the same verified candidate activates under the retained cache lock within the two-second recovery budget
- [~] 1.2 @e2e (agent) run staged Windows release acceptance with the exact candidate ZIP and real mise installation path -> defer: the exact shipping release runs only after merge; record its result as delivery verification without making it a pre-archive gate

## 2. Strict failure and ownership boundaries

- [ ] 2.1 @integration (agent) retain the real descendant blocker for the full recovery period -> activation stops within its bounded allowance, preserves the original typed access-denied error, private stage and source identity, and keeps the cache lock held until the blocking owner exits
- [ ] 2.2 @regression (agent) exercise moved-success, occupied or rebound destination, changed or unobservable source, observation failure, and non-access-denied errors -> only moved-success reconciles as success, while every unsafe or uncertain result stops without an activation retry and retains its original error
- [ ] 2.3 @integration (agent) cancel the async provisioning caller while Windows activation recovery is waiting -> the same owned future releases the stage, source and cache lock together without a later move, partial destination or competing-writer window

## 3. Repository verification

- [x] 3.1 @unit (agent) run focused memory filesystem and provisioning regressions, package typecheck, lint, formatting and strict Cospec validation -> host provision tests passed 7/7, the exact files regression passed 1/1, memory typecheck and lint passed, Rust formatting and diff checks passed, docs checks passed, and strict Cospec validation passed; Windows-only execution remains pending separately
- [x] 3.2 @runtime (agent) inspect the first PR native run at exact head `d8242fe95358db6428a613846f215f62a583124c` -> all four Windows coverage shards failed during compilation before runtime behavior, and the helper reported only its runner-local Cargo JSON path without rendering the compiler diagnostic
- [x] 3.3 @regression (agent) exercise `json-render-diagnostics` with a small isolated portable Cargo fixture -> Rust E0308 rendered on standard error while standard output retained parseable `build-finished` success false JSON with exit 101; after correcting the fixture, standard output contained the test executable `compiler-artifact` and `build-finished` success true with exit 0, and repository tooling lint passed (`/private/tmp/kuru-cargo-diagnostics.VNH77A`, `/private/tmp/kuru-activation-diagnostics-tooling.log`)
- [ ] 3.4 @integration (agent) observe the coverage helper's successful native CI inventory -> native executable inventory remains complete with diagnostic-bearing Cargo JSON and no deliberate failing PR build
- [ ] 3.5 @runtime (agent) run the ordinary native memory and installation shards, including the genuine optimized executable and ordinary native mise acceptance -> the first PR Windows installation job `104262450393` passed packaged cold offline install/update 1/1 in 81.23 seconds, but actual Windows activation-recovery regressions and the complete native gate remain pending after coverage compilation failed
