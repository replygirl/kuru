# Tasks

## 1. Release the key lock explicitly

- [x] 1.1 Add `files::release_lock` (unlock, then close) and verify it is the only release path of the template key lock in `creation_template.rs`
- [x] 1.2 Hold the key lock as `KeyLock`, whose drop releases through `release_lock`, through `try_key_lock`, `build`, `copy_blocking`, `ensure_in`, `create_in` and `quarantine`, and verify the crate typechecks with no bare key-lock `File` left in that module
- [x] 1.3 Release the key lock explicitly where it leaves the module as a reap guard (a failed template build in `stage_worker.rs`, the owner-drop reaper in `server.rs` after the reap) and verify `cancelled_build_releases_key_lock_only_after_reap` and `owner_drop_transfers_installed_reap_guard_until_real_child_reaps` pass

## 2. Regression tests

- [x] 2.1 Add the lock-contract test and the open-path test with a held duplicate of the shared key lock, and verify both fail with the unlock removed and pass with it
- [x] 2.2 Close an unexpectedly successful open before failing in the open-path tests that expect a failure, and verify the red open-path test reports only the unexpected success

## 3. Documentation and checks

- [x] 3.1 Document the explicit release in `docs/memory.md` and `docs/development.md` and verify `mise run docs:check` passes
- [x] 3.2 Run the checks in verification.md and record the evidence
