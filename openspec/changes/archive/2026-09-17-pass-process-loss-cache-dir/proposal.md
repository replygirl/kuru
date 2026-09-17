## Why

The process-loss fixture spawns its child with a fully cleared environment and only
forwards `KURU_DOLT_CACHE` when that variable happens to already be set in the parent's
own OS environment. In CI it is never set, so the child re-derives a cache directory
via `std::env::temp_dir()` with no `TMP`/`TEMP` present — on Windows that falls back to
the Windows directory, producing an unwritable, mismatched cache path distinct from the
one the parent's own `OpenOptions` already resolved and is asserting against.

## What Changes

- `packages/kuru-memory/src/store/recovery_tests.rs`: both `spawn_process_loss_creator`
  implementations (unix and windows) now take the parent's already-resolved
  `options.config.cache_dir` and set `KURU_DOLT_CACHE` in the child's environment
  explicitly, instead of conditionally passing through the parent process's own OS
  environment variable. Covers all three process-loss fixtures that share this spawn
  path (`fresh_staging_process_loss_after_ddl_is_preserved_and_never_reused`,
  `process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery`,
  `process_loss_child_cleanup_runs_after_failed_observation`).

## Impact

Test-only; no production path touched. Fixes a Windows-only `native-tests` CI failure
(`Access is denied (os error 5)` removing a private stage under
`C:\Windows\kuru-dolt-test-cache`); no coverage or CI-time change expected on other
platforms.
