## 1. Process-loss cache passthrough

- [x] 1.1 Pass the parent's resolved `options.config.cache_dir` explicitly as
  `KURU_DOLT_CACHE` in both `spawn_process_loss_creator` implementations
  (`packages/kuru-memory/src/store/recovery_tests.rs`, unix and windows) and verify
  by running `store::recovery_tests::fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused`
  locally (macOS) and confirming it passes. Verified: passes locally (macOS,
  `cargo test -p kuru-memory --lib`).
- [x] 1.2 Confirm the two sibling fixtures sharing the same spawn path
  (`process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery`,
  `process_loss_child_cleanup_runs_after_failed_observation`) still pass locally, run
  `cargo fmt -p kuru-memory -- --check` and `mise run //packages/kuru-memory:lint`,
  and record native Windows CI as the pending verification (this change cannot run
  Windows natively from this host). Verified locally: all four process-loss family
  tests pass (macOS), `cargo fmt -p kuru-memory -- --check` clean,
  `mise run //packages/kuru-memory:lint` clean. Windows native execution of the
  fixed `windows-2025` shard remains pending (not runnable on this host).
