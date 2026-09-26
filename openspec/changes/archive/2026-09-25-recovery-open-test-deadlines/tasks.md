## 1. Recovery fixture observation bounds

- [x] 1.1 Use the existing configured migration observation deadline for full cold recovery and reopen waits in `packages/kuru-memory/src/store/recovery_tests.rs`, retaining short fault and readiness waits and all state assertions.
- [x] 1.2 Run the focused process-loss and lost-reply recovery tests on the corrected source, then record the actual result and keep exact-head native CI as a before-merge gate.
- [x] 1.3 After review, derive the six first-boundary waits that include a fresh Dolt start (lost commit, branch, fast-forward and absent-publish routed sources; accepted-cancellation first boundary; process-loss contender takeover) from the same deadline, keep post-`resume_route()` waits short, record why the absent-refusal and usage-upgrade waits stay short, and rerun the recovery module.

## Observed evidence

Run via `mise run //packages/kuru-memory:test -- <filter>` (real Dolt, `KURU_TEST_SUPERVISOR_PREPARED=1`), local macOS (aarch64-apple-darwin), 2026-09-25:

- `store::recovery_tests::process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery ... ok` (1 passed; 7.46s)
- `store::recovery_tests::production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends ... ok`
- `store::recovery_tests::production_upgrade_reconciles_lost_branch_reply_after_exact_ref_creation ... ok`
- `store::recovery_tests::production_upgrade_reconciles_lost_fast_forward_reply_after_target_publication ... ok`
  (3 passed; 14.46s)

Review follow-up on branch `fix/restore-green-main` (13 waits now use the
derived deadline), `mise run //packages/kuru-memory:test -- --lib store::recovery_tests`,
local macOS (aarch64-apple-darwin), 2026-09-25:

- `store::recovery_tests`: 23 passed, 0 failed (96.72s), including every
  fixture whose waits changed. This host stays inside the old 10s bound, so it
  shows the derived waits do not break the fixtures; it does not reproduce the
  slow-runner failure.
- `cargo fmt -p kuru-memory --check` and `mise run //packages/kuru-memory:lint`: clean.

Not run (named explicitly, per AGENTS.md — never manufacture a pass):
- Full `kuru-memory` coverage suite and workspace coverage gate — out of scope
  for this focused change per the harness instructions; the lead serializes
  the full coverage run before push.
- Exact-head native CI (macOS-Intel `native-build`, Windows shards) — requires
  a push, which this change does not perform; remains the before-merge gate
  per `tasks.md` 1.2's original wording.
- The unrelated Windows `memory-runtime` hang
  (`tests::authorized_web_fetch_overlaps_checked_search_and_keeps_result_order`,
  `packages/kuru-runtime/src/tests.rs:1059`) is Track B of the audit and is not
  touched or claimed fixed by this change.
