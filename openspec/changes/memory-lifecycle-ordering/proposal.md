# Proposal

## Why

The memory service lets two lifecycle steps run before the state they depend on has ended, and Dolt refuses or panics when that happens. Both were found by investigating the macOS 26 uncertain-write flakes and confirmed by code reading plus a deterministic injected test.

**H1: candidate cleanup deletes a branch before its sessions have ended.**

- Dolt v2.3.5 (`dolt_branch.go`, `validateBranchNotActiveInAnySession`) refuses any checked `DOLT_BRANCH` delete or rename with error 1105 while another server session still has that branch checked out. Idle pooled sessions count.
- A closed client session stops counting only when the server's per-connection goroutine leaves its read loop. That happens asynchronously from the client's close. The valid signal is the connection disappearing from `information_schema.processlist`.
- The candidate rename is gated. `transition_candidate_with_retirement_deadline` runs `fence_pool`, `retire_pool`, then `await_branch_sessions_end(.., QUERY_TIMEOUT)` before `DOLT_BRANCH('-m')`.
- The candidate delete is not gated. `delete_candidate_ref` runs `retire_pool`, takes an owned connection, then issues `DOLT_BRANCH('-d'|'-D')`. Immediately before it, `candidate_branch_is_clean` opens and closes a fresh pool on that branch, so a just-closed session is always present. On the abandon path, `confirm_no_live_candidate_session` does a checked self-rename that hits the same check.
- Deterministic reproduction: with one injected lingering session (20, 100 or 1000 ms), the ungated delete fails in 3-4 ms with exactly `candidate_owner stage=cleanup class=database sqlstate=HY000 vendor=1105 reason=other`, 15/15. The gated rename waits the same linger out and succeeds, 15/15.
- The service maps that cleanup error to `StorageFailed`, and the client then fences itself with "memory service write outcome is uncertain; further client mutations are blocked". That is the record of catalogue items 5, 8a and 12.
- The canary `candidate_pool_retirement_observes_exact_server_sessions_before_rename` asserts that no session remains after pool retirement. The product never guaranteed that, and the canary failed in item 11.
- **Status: fault proven by code and injected test; natural trigger observed on CI (macOS 26, 3 vCPU), not reproduced on the 14-core local host.** The CI canary printed `active_sessions=2` after pool close. Locally, 0 of 1,200 checked deletes and 0 of 2,400 product abandons failed, idle and under 14-core saturation.

**H2: a Dolt data directory disappears before its server has exited.**

- Dolt panics at close with `fatal error closing table persister` when its data directory disappears before it exits. The signatures are "new manifest created with non 0 lock" and a missing or unrenameable `nbs_manifest_*`. This is upstream issue dolthub/dolt#10971, still open.
- Local before-baseline:
  - A 100-iteration kuru-runtime loop started 895 servers, and 169 of them panicked at close. 132 service supervisors exited unsuccessfully.
  - Full suites: kuru-memory 1/778 and 2/782 servers panicked; kuru-runtime 2/337.
  - None of the 196 affected servers followed a product directory move or lease removal.
- There are two classes:
  - **A (product):** `MemoryStore::open` error paths reached after the server started return without awaiting the server's bounded close. The server goes to `Owner::drop`'s background reaper, and the caller then removes the directory.
  - **B (fixtures):** runtime fixtures delete their data directory right after the client closes. The managed service deliberately keeps running until its 30 s idle grace, so its supervisor is still stopping Dolt while the tree is removed.

**Windows (catalogue items 1 and 6).**

- `StagedActivation` has explicit, checked teardowns for publication (`finish_published`) and failure (`retain`).
- A cancelled activation has no such teardown. It drops field by field, so its `PrivateTemp` is removed through `tempfile::TempDir::drop`, which swallows a failed `remove_dir_all`.
- A transient Windows handle hold therefore leaves the stage present, and the stage was never reported.
- **Not fixed by this change.** The teardown is split into its own change (branch `fix/memory-provision-stage-teardown`); see What Changes.

## What Changes

**H1: session release before candidate ref deletion.**

- Candidate pool retirement waits for server-observed session end before any candidate ref is deleted or renamed. This covers:
  - the `-d` delete in `cleanup_promoted_candidate`
  - the `-m b b` exclusion probe and the `-D` delete in `cleanup_abandoned_candidate`
  - the existing rename
- The observation is the one the rename already uses: `information_schema.processlist` has no session on `kuru/<branch>`.
- It is bounded by the existing `QUERY_TIMEOUT` retirement deadline and tagged `CandidateFailureStage::PoolRetirement`.
- Nothing is retried, no deadline is raised, and no `--force`/`-D`-as-force is used to bypass the check.
- A session that never ends still fails the operation, and the uncertain-write fence still trips.

**H2-A: open error paths await the bounded close.**

- Every `MemoryStore::open` error return after `Server::open*` succeeded awaits the owned server's bounded close before returning. The mechanism is the existing `close_installed_guard` / `close_migration_worker`, bounded by `CLOSE_GRACE` and `SUPERVISOR_REAP_ALLOWANCE`.
- A close failure is attached as context to the original error.
- `open()` returning `Err` therefore means the engine has been reaped, or its reap is still held by the lifecycle authority under the existing bounded-close contract.

**H2-B: fixtures retire the service before releasing the directory.**

- Fixtures that own a managed service's directory retire the service before they release that directory. The mechanism is the existing `test_support::retire_idle_service`, then lifecycle quiescence.
- A test-support fixture invariant FAILS any test whose store directory is removed while its Dolt server is still alive, instead of silently deleting it.

**Windows `StagedActivation`: split out, not in this change.**

- This change has three parts (H1, H2-A, H2-B). The Windows cancelled-activation teardown (catalogue items 1 and 6) was drafted but never type-checked or run on Windows, and by static reading it makes the frozen test `cancelling_checked_activation_recovery_drops_stage_before_cache_lock` fail deterministically: that test's own `_parent` handle keeps `runtime` delete-pending under kuru-platform's legacy delete disposition. Resolving that fixture handle without weakening its assertion needs a maintainer decision.
- The draft is parked, unmerged, as one commit on branch `fix/memory-provision-stage-teardown` (worktree `tmp/worktrees/fix-memory-provision-stage-teardown`, cut from origin/main c986f4ff) and needs its own cospec change. Items 1 and 6 stay open until that change lands.
- Splitting is an implementer decision taken pending maintainer confirmation; the alternative is to fold the parked commit back in once the fixture-handle question is settled.

**Diagnostics only.** `candidate_branch_rename_reason` also labels the exact upstream in-use message as `branch_in_use` at `Cleanup`. This changes only the test/test-support failure record. It does not change what the client is told.

**Unchanged.** No retries, no raised deadlines or timeouts, no `--force`, no excluded tests, no weakened assertions. The uncertain-write fence is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living specs already require the corrected behaviour; only the implementation was wrong:

- `versioned-memory`: "Candidate branches MUST be reclaimed only after ... every relevant SQL session has ended" (H1), and "retain ownership until its child is reaped" (H2-A).
- `project-memory-service`: "close pools, await its owned supervisor and Dolt reap, retire its endpoint, and release leases in that order" (H2-B relies on this contract; it is not changed).

## Impact

- `packages/kuru-memory/src/store.rs`:
  - the candidate cleanup ordering (`delete_candidate_ref`, `confirm_no_live_candidate_session`, the retirement step)
  - `MemoryStore::open` error paths
  - the `candidate_branch_rename_reason` diagnostic label
- `packages/kuru-memory/src/server.rs`: test/test-support lifecycle trace instrumentation, and `SUPERVISOR_REAP_ALLOWANCE` exposed `pub(crate)` so test support reuses it instead of a copy.
- `packages/kuru-memory/src/test_support.rs` and `src/test_support/*`: the fixture-directory invariant, and a managed-quiescence helper built on `retire_idle_service`.
- `packages/kuru-runtime/src/dream.rs` and the other managed-store fixtures the invariant identifies.
- `apps/kuru-tui/src/ui/runtime_tests.rs`: a store fixture found by the round-2 Dolt traces (its root was a plain `tempfile` directory the invariant does not check).
- New regression tests in kuru-memory. The branch's existing measurement support (`store/lifecycle_measurement_tests.rs`, `test_support/lifecycle_trace.rs`) is retained as ignored measurement code.
- `packages/kuru-memory/src/service/rpc.rs` (the test-only paused exchange), `spawn_gate.rs`, `test_support/served_owner.rs`, and the lost-reply and owner-restart tests in `facade.rs`, `service.rs`, `store.rs` and `store/migrations.rs`, plus kuru-runtime `hook_tests.rs`: two fixture ordering rules (design D7, D8). Test support and tests only.
- `docs/development.md`: the fixture teardown invariant and the two fixture ordering rules.
- No public API, configuration, protocol, schema or user-visible behaviour change. No workflow change.

## Surfaces

None is touched. This is an internal ordering fix in the memory service and in test fixtures. It changes no UI, CI topology, external contract or agent behaviour.

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
