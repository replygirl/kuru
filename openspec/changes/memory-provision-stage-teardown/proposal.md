## Why

`kuru-memory` installs the bundled engine in a private stage (`PrivateTemp`) under the exclusive installation lock (`CacheLock`) in `packages/kuru-memory/src/provision.rs`. Nothing owned the order in which those two are released: `StagedActivation` relied on field declaration order, the extraction-error return (`let (staging, lock, extraction) = …; extraction?`) drops its bindings in reverse order and so released the lock *before* the stage (observed with a drop-order probe), and every implicit exit removed the stage through `tempfile::TempDir::drop`, which silently discards a failed removal. On Windows, where a transient or persistent handle inside the stage refuses deletion, a cancelled or failed installation could release the lock with its stage still on disk and nothing reported or recorded. After a successful publication the retained-stage receipt was written only after the lock had been released (review finding S5).

Separately, Windows activation recovery could end without its recovery context, and a caller cancelled at that point received a completed failure. When the first checked no-move result (access denied, source held, destination absent) returned after the two-second recovery window had already elapsed, the loop returned that first error bare, without the "runtime activation recovery stopped" context, and with no await between observing the no-move and returning, so a caller cancelled at that point received a completed failure instead of a cancellation.

The flaky-test catalogue (`tmp/roadmap/phase2-handoff-2026-09-26-sources/flakes/flaky-tests.md`) records three windows-latest failures of `kuru-memory` library tests that come from two separate mechanisms:

- Item 6, stage teardown (M1): `provision::native_tests::cancelling_checked_activation_recovery_drops_stage_before_cache_lock` panicked at `native_tests.rs:938` with "cancellation drops the private stage before releasing the cache lock" (PR #115, run 36292320870, job 108544879759). The task was cancelled, but the stage was still present after `TempDir::drop` ran.
- Item 1, first attempt past the window (M2): `provision::native_tests::persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage` failed `diagnostic.contains("runtime activation recovery stopped")` at `native_tests.rs:802` (PR #110, run 36270828072, job 108484586358). Its earlier assertions had passed: elapsed ≥ `ACTIVATION_RETRY_LIMIT`, at least one checked denial, `Rejected`, os error 5.
- New occurrence, first attempt past the window (M2): the same cancellation test panicked at `native_tests.rs:947:24`, `task.await.unwrap_err()`, on an `Ok` value: `Err(verified Dolt activation failed; preserved private stage at …\.install-724H4l\private` caused by `activate verified Dolt runtime` → `memory directory reconciliation proved the held source remains and the destination is absent` → `Rejected publication during native-move at …\active: Access is denied. (os error 5)`) (PR #124, run 36407104338, job 108878373677). The chain carries no recovery context, so the first attempt was terminal. With os error 5 that happens only through the `now >= deadline` branch. The task finished instead of being cancelled.

## What Changes

- New `StageLease` in `provision.rs` owns the private stage, the installation lock and the asset it installs. It has four explicit exits, and each one releases the lock last:
  - `close_published`, after a verified publication: checked removal (`PrivateTemp::close_or_keep`). If removal is refused or uncertain, the lease writes the `.leftovers` receipt and emits the diagnostic, then releases the lock.
  - `discard_after`, on an error before publication: the same sequence with `published: false`. The caller's error also names the retained stage.
  - `keep`, when activation failed: the stage is kept as evidence, named in the returned error ("preserved private stage at"), then the lock is released. This is unchanged.
  - `Drop`, on cancellation or unwind: behaves as `discard_after` without an error to annotate.
- Ordering contract: once the lease owns a created stage, on every path (publication, error, cancellation, recovery exhaustion), the installation lock is released only after the stage is gone, or after its retention has been reported (diagnostic) and recorded (receipt), or, for activation evidence, after it has been deliberately preserved and named in the returned error.
- `StageLease::drop` blocks its thread for at most the existing `CLEANUP_RETRY_LIMIT` on Windows, or one `remove_dir_all` on Unix. It never blocks without bound.
- Windows activation recovery: a recoverable (os error 5) no-move that arrives after the window closed, even the first, now reports "runtime activation recovery stopped after N retries before starting another native move" instead of the bare first error, and preserves and names the stage before the lock is released. Cancellation is honoured at recovery's existing retry-spacing wait; no await point is added (design.md decision 5 records the ruling that dropped an earlier `yield_now`). The window, its spacing and the retry rule are unchanged.
- The frozen cancellation test keeps every assertion. Its fixture now releases the `runtime` directory handle returned by `files::read` together with the LICENSES blocker, which it already released. That handle kept `runtime` delete-pending under the checked (legacy-disposition) removal, which the unchecked `TempDir::drop` had bypassed. This fixture change is flagged for review in design.md.
- Not changed: `ACTIVATION_RETRY_LIMIT`, `ACTIVATION_RETRY_SPACING`, `CLEANUP_RETRY_LIMIT`, `CLEANUP_RETRY_SPACING`, `LOCK_TIMEOUT`, which errors are recoverable, evidence retention on activation failure, and the sweep (it already collects any receipted `.install-*` stage). No test assertion, timeout or retry limit is changed, and no retry loop is added.

## Capabilities

### New Capabilities

### Modified Capabilities

- `embedded-runtime`: adds the install-stage teardown ordering requirement. The living "Safe local extraction" requirement names cancellation cleanup but does not say what may happen to the lock when cleanup fails.
- `native-windows`: adds the activation recovery boundary requirement. The living recovery requirement does not cover a first checked result that arrives after the window, or cancellation at a checked no-move.

## Impact

- `packages/kuru-memory/src/provision.rs`: `StageLease`; `provision_with_extractor_observed`, `CheckedColdProbe::probe`, `StagedActivation` and `activate_staged_*` use the lease. `record_retained_stage` takes the publication flag. `emit_retained_stage_diagnostic` has a distinct message for unpublished stages. The Windows recovery loop changes. A `#[cfg(test)]` path-scoped seam (`observe_retained_stages`) lets tests observe each retention report before the lease releases the lock.
- `packages/kuru-memory/src/provision/tests.rs` (Unix): regression tests for extraction-error retention, cancellation retention and the published receipt-before-release order, observed through a test-only retained-stage seam.
- `packages/kuru-memory/src/provision/native_tests.rs`: Windows regression tests for cancelled activation with a held stage (M1) and for a first checked result after the window, with and without a pending cancellation (M2). Existing call sites move to the lease and report types. The frozen cancellation test's fixture handle is released, and its tokio clock is paused around the activation so a slow first move cannot close the window before the cancellation reaches the retry-spacing wait (flagged).
- `docs/memory.md`: retained unpublished stages and the lock order.
- Follow-on, not changed here: `CacheLock` is still released by descriptor close. The #124 audit already lists it for the shared explicit-release guard.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
