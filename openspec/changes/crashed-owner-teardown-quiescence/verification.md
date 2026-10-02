# Verification

## 1. A body error surfaces instead of being replaced by the guard's panic [critical]

- [ ] 1.1 @regression (agent) run the task-2.1 fixture (real owner spawn, SIGKILL, injected `bail!` before `await_managed_quiescence`) against the unmodified fixture -> it panics with the guard's "no quiescence record" message, not the injected text, proving the defect
- [ ] 1.2 @regression (agent) run the same fixture after the task-3 restructure -> the test fails with the injected failure text as the primary error, with no bare guard panic

## 2. The fixture root is actually released once the killed owner quiesces

- [ ] 2.1 @integration (agent) after 1.2, inspect the fixture's `TempDir` path -> it no longer exists (released, not kept), confirming `await_managed_quiescence` ran before `root.release(outcome)`
- [ ] 2.2 @unit (agent) read `test_support::retire_idle_service`'s handling of an owner that already exited without a current endpoint record -> it returns `Ok` rather than erroring, so a best-effort quiescence attempt on an error path does not mask the body's real error

## 3. The original fixture still proves the product's crash-recovery contract

- [ ] 3.1 @regression (agent) run the restructured `crashed_owner_retains_accepted_receipt_after_sibling_write` on its happy path -> it passes, and `await_managed_quiescence` still runs exactly once on success
- [ ] 3.2 @manual (agent) if a local run cannot complete this session (shared build cache contention, busy machine) -> defer: name the exact blocking condition observed

## 4. No product-code regression

- [ ] 4.1 @unit (agent) diff this change against `packages/kuru-memory/src/server.rs`, `attach_or_spawn_elected`, `ORDINARY_POOL_WINDOW`, and the supervisor readiness deadline -> no lines changed in any of them
