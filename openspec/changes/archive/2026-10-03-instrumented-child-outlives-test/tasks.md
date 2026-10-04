# Tasks

## 1. Diagnose the late profile

- [x] 1.1 Read run 37164260608 partition 8's job log and diagnostics artifact and verify the window: the last executable (`kuru_runtime` lib, 35 tests) ended 00:29:39.06, the profile set was taken 00:29:39.15 and the new profile was found at 00:29:45.22; the artifact holds profile counts only, no names
- [x] 1.2 Reproduce the partition's `kuru_runtime` and `kuru_memory` test sets locally under instrumentation, as the runner does (own process group, group SIGKILL after the root exits), and verify which binary and test leave a profile after the test process exits

## 2. Fix the leaking test

- [x] 2.1 Add `test_support::{supervisor_mark, unawaited_supervisors}` over the engine ledger, recording each owner dropped live, and verify `only_a_close_awaits_a_marked_tests_supervisors` passes
- [x] 2.2 Regression test: `provider_free_undo_preserves_sessions_and_archives_added_identities` asserts no unawaited supervisor; verify it fails before the fix (store still live, or dropped without a close) and passes once the test closes its store

## 3. Name the writer of a late profile

- [x] 3.1 Append spawn rows to the runner ledger from memory supervisor and owner spawns and terminal children (`test_support::spawn_ledger`), forward the ledger to owners and terminal children, and verify rows name the originating test
- [x] 3.2 Have the runner set `KURU_COVERAGE_SPAWN_LEDGER` on each test process and record each listing profile, parse spawn rows apart from runner records, and verify the dispatch test's rows
- [x] 3.3 Attribute each changed profile in the partition's invariant failure by pid, ancestry and module signature, and verify `a_late_profile_names_the_test_that_started_its_process`

## 4. Documentation and checks

- [x] 4.1 Describe spawn rows, attribution and the close requirement in `docs/development.md` and verify `docs:check`
- [x] 4.2 Run the narrowed package tests three times and the static checks, and record them in verification.md
