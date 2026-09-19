## 1. Non-fatal completion and receipt (U-receipt)

- [x] 1.1 Add `StageCleanupExhausted { attempts, elapsed }` as the typed context of the bounded cleanup exhaustion in `packages/kuru-memory/src/files.rs` and verify the rendered message is byte-identical to today's.
- [x] 1.2 Add `PrivateTemp::close_or_keep` returning `Result<(), StageCleanupFailure>` and verify a failing close retains the stage exactly as `keep` does, with no second removal attempt.
- [x] 1.3 Return the typed cleanup outcome from `finish_published`, `activate_staged_with` and `activate_staged_after_probe`, and verify the retain/failure paths and publication outcomes are unchanged. The threaded value is `Option<StageCleanupFailure>`; the caller at `provision.rs` owns the asset digest and target and builds the `StageCleanupReport` there.
- [x] 1.4 Write the private JSON receipt to `versions/.leftovers/<stage>.json` through the existing atomic private write, and verify the file exists with owner-only permissions after a forced exhaustion.
- [x] 1.5 Add the `#[cfg(test)]` removal-failure hook in `files.rs` and verify it changes no product code path.
- [x] 1.6 @regression Add the unix fixture that forces exhaustion and verify publication succeeds, the engine runs, the receipt is written and the stage is retained.

## 2. Sweep and cap (U-sweep)

- [ ] 2.1 Add `sweep_leftover_stages(versions, &CacheLock)` using the existing checked `remove_tree` and verify it removes only receipted `.install-*` directories.
- [ ] 2.2 Leave stage and receipt in place on any `Rejected` or `Uncertain` removal and verify nothing is deleted on uncertainty.
- [ ] 2.3 Call the sweep on the cold path after the installation lock is acquired and verify a pre-existing receipted stage is collected before the new stage is created.
- [ ] 2.4 Add the non-blocking `try_cache_lock` and the receipts-present warm-open sweep and verify warm opens take no lock when `versions/.leftovers` is absent and skip the sweep when the lock is busy.
- [ ] 2.5 Add the documented count-based `LEFTOVER_STAGE_CAP` and verify reaching it only reports and never deletes.

## 3. Notice and documentation (U-notice-docs)

- [ ] 3.1 Add `MemoryOpenStage::RetainedInstallStage`, widen the progress bitmask to `u16` and the channel to 16, and verify every existing stage still reports exactly once.
- [ ] 3.2 Add the stderr notice line after `Memory: ready.` in `apps/kuru-tui/src/cli.rs` and verify it is emitted only on a successful open and that stdout stays JSON-clean under `run --json`.
- [ ] 3.3 Emit the typed report as one structured `tracing::warn!` and verify it reaches the diagnostics ring and is neither model-visible nor stored in memory.
- [ ] 3.4 Add the growth sentence to `docs/memory.md` and its `apps/kuru-docs/concepts/memory.md` mirror and verify `docs:check` passes.

## 4. Verification

- [ ] 4.1 Run scoped memory and TUI format, lint, typecheck and behavioral tests and record observed results.
- [ ] 4.2 Record Windows-native execution status explicitly, obtain independent source review, then archive the change.
