# Design

## Context

### What the CI logs show

The three catalogued windows-latest failures have two different mechanisms. The job logs establish this directly.

- **M1, teardown swallowed** (item 6; PR #115, job 108544879759). The assertion `!stage_path.exists()` failed ("cancellation drops the private stage before releasing the cache lock"), so the task *was* cancelled. On main, cancellation drops `StagedActivation` in field order: source handle, probe, `PrivateTemp`, `CacheLock`. `PrivateTemp` drops its `tempfile::TempDir`, whose `Drop` calls `remove_dir_all` and discards any error. A transient Windows handle inside the stage (antivirus or the indexer on the just-extracted `dolt.exe`, or the image section of the just-run probe) makes that removal fail. The lock is then released with the stage still present, and nothing records it.
- **M2, first result after the window** (item 1: PR #110, job 108484586358; new occurrence: PR #124, job 108878373677). Job 108878373677 panicked at `native_tests.rs:947:24` on `task.await.unwrap_err()`, which received `Ok(Err(..))`. The task completed rather than being cancelled. Its error chain is `verified Dolt activation failed; preserved private stage` → `activate verified Dolt runtime` → `memory directory reconciliation proved the held source remains and the destination is absent` → `Rejected … Access is denied. (os error 5)`, with **no** `runtime activation recovery stopped` context. In the main Windows loop, a proven no-move with os error 5 skips that context only when `first_error` is still `None`, which means the first attempt, and `now >= deadline`. So the first `activate_once` returned only after `ACTIVATION_RETRY_LIMIT` had elapsed. `observer(true)` (which aborted) was followed by `retain` and a return with no await, so the abort never landed. Item 1 has the same shape. Its assertions that elapsed ≥ the window, that `checked_denials >= 1`, `Rejected` and os error 5 all passed, and then `contains("runtime activation recovery stopped")` failed. The logs have no per-test timing, so the ≥2 s first attempt is inferred: the error chain allows no other path. It was not measured.

### Other ordering defects found by reading

- Extraction error: `let (staging, lock, extraction) = …; extraction?;` drops the pattern's bindings in reverse order, so `lock` is released before `staging` is removed. A local probe (`droporder.rs`, compiled with the pinned toolchain) printed `drop lock` then `drop staging`.
- Published path (review S5): `finish_published` released the lock, and the caller wrote the `.leftovers` receipt only afterwards.
- `CheckedColdProbe::probe` and the probe-preparation error path dropped the stage through `TempDir::drop`, so a failed removal was discarded there too.

### Removal semantics

`PrivateTemp::close` removes the stage through `kuru_platform`'s checked `Directory::remove_tree` inside the existing bounded recovery (`close_windows_private_stage`, `CLEANUP_RETRY_LIMIT` = 2 s) on Windows, and through `TempDir::close`, which reports its error, on Unix. The Windows checked removal deliberately uses the legacy `FileDispositionInfo` and never POSIX unlink (`packages/kuru-platform/src/fs/windows.rs`). A share-delete handle on a descendant therefore leaves that name delete-pending until the handle closes, and its parent cannot be removed until then. `std::fs::remove_dir_all`, which `TempDir::drop` uses, removes the name immediately with POSIX semantics.

## Goals / Non-Goals

**Goals:**
- One owner (`StageLease`) orders stage resolution before lock release on every path. Stage resolution is removal, reported and receipted retention, or named evidence.
- A cancelled Windows activation at a checked no-move is always cancelled, and recovery that ends on a recoverable denial is always reported as stopped recovery.
- Deterministic regression tests: Unix variants run locally; `cfg(windows)` variants run in the windows-latest and windows-11-arm partitions.

**Non-Goals:**
- Changing any deadline, spacing, retry limit or recoverable-error set, or adding a retry loop or sleep.
- Changing `CacheLock` release-by-close. The #124 audit lists it as a follow-on for the shared explicit-release guard.
- Changing the Unix removal mechanism, which stays `TempDir::close`, or the sweep.

## Decisions

1. **The lease records retention; callers do not.** `StageLease { staging, lock, asset }` derives the version directory from the stage container's parent, so the receipt's `relative_to` check holds both in production and in the fixtures under `cache`. `close_published` and `discard` share one `release(published)` path. That path runs `close_or_keep` and, on failure, adds context, runs `record_retained_stage` (the receipt) and `emit_retained_stage_diagnostic`, and only then drops the lock. The caller receives an `Option<StageCleanupReport>` and only maps it to a progress stage. *Rejected:* returning a failure value that carries the lock (the draft's `StageTeardownFailure`). That hands the ordering back to every caller and let the published path release before the receipt.
2. **Unpublished retention is receipted.** A stage left by an error or a cancellation is disposable, and the sweep already collects any receipted `.install-*` stage under the lock. The receipt carries `published: false`, and the diagnostic uses a distinct message with the same admitted fields. *Rejected:* diagnostic only. That leaves an uncollectable stage behind on every Windows handle refusal.
3. **Evidence stays evidence.** `keep` (activation failure: occupied destination, a non-recoverable error, recovery exhausted) is unchanged. The stage is preserved on purpose and the returned error names it. This is the third resolution in the contract, not an exception to it.
4. **`Drop` tears down synchronously within the existing bound.** The frozen cancellation test requires the stage to be gone when the `JoinHandle` resolves, so teardown must run as the future is dropped. Moving it to `spawn_blocking` would race that and is unavailable during runtime shutdown. `block_in_place` panics on a current-thread runtime. The previous `TempDir::drop` was synchronous too. The bound is one `remove_dir_all` on Unix and `CLEANUP_RETRY_LIMIT` plus a receipt write on Windows, and only when a handle refuses removal. That bounded block is the documented trade-off (reviewer point on `StageLease::drop`).
5. **Every checked no-move is a cancellation point (review point).** In the Windows loop, `tokio::task::yield_now().await` runs after `observer(true)` and before the retry or stop decision. `sleep` cannot provide this: a tokio timer whose deadline has already passed returns `Ready` on its first poll without yielding. The yield is not a retry, not a delay, and not a deadline change. It makes cancellation deterministic regardless of how long the attempt took, and a cancelled activation then gets lease teardown instead of evidence retention.
6. **A late first recoverable result reports stopped recovery.** If the first recoverable no-move arrives at or after the deadline, the loop returns `expired_activation_error(error, retries)` ("runtime activation recovery stopped after 0 retries before starting another native move"), which is exactly what happened. This preserves the typed `PublicationError` (still downcastable) and os error 5.
7. **Frozen test fixture handle (flagged for review).** `cancelling_checked_activation_recovery_drops_stage_before_cache_lock` held `_parent`, the `runtime` directory handle returned by `files::read`, until the test ended. Under the checked legacy-disposition removal, that handle keeps `runtime` delete-pending, so `private` cannot be emptied, the 2 s window runs out and the stage is retained. The test would then fail deterministically. The old unchecked POSIX `remove_dir_all` bypassed the handle. That incidental handle is not the scenario under test. The smallest change that keeps the test's intent moves it into the observer closure together with `blocker` and drops both on the first proven no-move. Every assertion is unchanged, and so is the order of the blocker release and the abort.

## Risks / Trade-offs

- [A persistent holder longer than `CLEANUP_RETRY_LIMIT` during a cancelled Windows activation still leaves the stage on disk] → That is now reported and receipted rather than hidden. The frozen test would still fail if a scanner held a handle for more than 2 s, which is the existing bound this change must not raise.
- [The runtime worker blocks for up to 2 s when a cancelled Windows activation meets a refusing handle] → Bounded, rare, and the same class of blocking as the previous synchronous `TempDir::drop`.
- [The M2 mechanism is inferred from the error chain, not timed] → The new Windows regression tests reproduce it deterministically by holding the observer past `ACTIVATION_RETRY_LIMIT`, which is a fixture delay rather than a product one.
- [`cfg(windows)` code cannot be compiled on the macOS authoring host] → windows-latest and windows-11-arm partitions compile and run it. They are named in verification.md as unrun locally.
