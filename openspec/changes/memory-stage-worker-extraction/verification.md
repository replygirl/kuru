# Verification

## 1. Fresh-open engine/lock/lease sequence is unchanged [critical]

- [ ] 1.1 @equivalence (agent) run existing marker-boundary, recovery and preserve_unready_stage kuru-memory tests unchanged -> all pass, no assertion edits; a failure names the diverging step
- [ ] 1.2 @regression (agent) run the existing engine_ledger-based fresh-open count assertion -> 4 engine starts, 3 closes, 10 process launches, matching today
- [ ] 1.3 @integration (agent) drive one real fresh MemoryStore::open via temporary_cold() against a real Dolt supervisor -> Ready, with schema version, initial_revision and ready.json shape identical to pre-extraction

## 2. Cancellation during stage build still reaps before the lock returns [critical]

- [ ] 2.1 @integration (agent) run cancelled_open_during_stage_build_keeps_startup_lock_until_reap: cancel an opener mid StageWorker job, then start a second opener on the same project -> the second opener's startup-lock acquisition completes strictly after engine_ledger records the first engine's reap

## 3. Creation selector reaches no product caller

- [ ] 3.1 @unit (agent) cargo check -p kuru-memory --features test-support with the new creation field on OpenOptions -> compiles
- [ ] 3.2 @regression (agent) grep -rn "OpenOptions {" packages/ apps/ excluding store.rs and test_support.rs -> no matches
- [ ] 3.3 @equivalence (agent) run temporary(), temporary_cold() and open_temporary() at their existing 15 call sites -> all still compile and open successfully, unchanged from before this change

## 4. Lint and type checks stay clean

- [ ] 4.1 @unit (agent) run kuru-memory lint and typecheck, including the Windows lint target -> clean pass, no new warnings from the module split
