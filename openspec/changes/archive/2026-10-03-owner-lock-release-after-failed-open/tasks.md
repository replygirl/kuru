# Tasks

## 1. Regression test

- [x] 1.1 Add the test-only `OwnerHooks::duplicate_owner_lock` and `a_failed_open_releases_its_owner_lock_despite_a_duplicate_descriptor`, which holds a duplicate of the owner lock across a failed open, and verify it fails before the fix

## 2. Fix

- [x] 2.1 Give `ServiceLock` a `Drop` that unlocks through `files::release_lock`, with `release()` taking the handle so it unlocks once, and verify the regression test and the service tests pass
- [x] 2.2 Release the owner lock explicitly on the failed open's publication path after `close_store_and_record`, and verify the existing activity tests pass unchanged

- [x] 2.3 Declare `MaintenancePermit`'s owner lock before its start lock so its drop releases them in the reverse of their acquisition, and verify the service tests pass

## 3. Docs and checks

- [x] 3.1 State the service-lock release rule in `docs/development.md` and verify `mise run docs:check`
- [x] 3.2 Run the full kuru-memory suite, the activity tests five times, and the static checks, and record the results in verification.md
