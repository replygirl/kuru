# Tasks

## 1. Regression test

- [x] 1.1 Add `model_catalog_tests::failed_model_catalog_is_retried_and_warned_once_per_session` with a fake provider whose `models()` fails twice then advertises a context window, and a process-wide `kuru.runtime` warning recorder; verify it fails against the unmodified `get_or_init` site

## 2. Fix

- [x] 2.1 Fill the catalog cell with `get_or_try_init`, proceed with an empty catalog on error, and warn once per harness through `model_catalog_warned`; verify the regression test passes
- [x] 2.2 Confirm the unguarded warning would be caught: temporarily warn on every failure and verify the test fails on two records, then restore

- [x] 2.3 Gate retries behind a 30 s retry instant set on failure, checked inside the initializer so callers queued behind a failed attempt use the fallback; add `concurrent_lookups_share_one_slow_failed_listing` and verify it fails with the gate disabled

## 3. Checks

- [x] 3.1 Run `mise run //packages/kuru-runtime:test`, `format:check`, `lint`, `lint:windows`, `typecheck` and `cospec -- validate --all --strict`, and record exit codes in verification.md
