# Verification

## 1. A cancelled dream leaves no member of the hook's owned group [critical]

- [x] 1.1 @regression (agent) `hook_tests::cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants` with `kuru-platform` `signal_group` temporarily replaced by a no-op returning `Sent` (root still killed and reaped, group never signalled) -> observed 2026-10-03 macOS arm64: FAILED at hook_tests.rs:1793 with `hook descendant survived dream cancellation: survivors [pid=84705 ppid=1 pgid=84702 … /bin/sh -c …; pid=84706 ppid=1 pgid=84702 … sleep 30; pid=84708 ppid=84705 pgid=84702 … sleep 30]; descendant outlived its 30 s sleep: false`; the mutation was reverted from a saved copy (`git status` shows only `hook_tests.rs` changed), then the test passed (1 passed)
- [x] 1.2 @integration (agent) group membership of a backgrounded subshell: `perl -e 'setpgrp(0,0); exec "/bin/sh","-c", q{(sleep 1; :) & … ps -o pid,pgid,ppid -g $$}'` -> observed: the subshell and its `sleep` carry PGID equal to the root pid (`$$`)
- [~] 1.3 @regression (agent) the unchanged test, five runs under 42 concurrent `yes` processes on 14 cores -> defer: did not reproduce locally (5/5 passed, 6.4 s to 10.1 s); the CI failure is inferred to be cancel-to-reap latency above the descendant's `sleep 1` on a loaded, coverage-instrumented macos-latest runner, not measured

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-runtime:test -- hook_tests`, five times -> observed: 22 passed, 0 failed each run (17.8 s to 26.1 s)
- [x] 2.2 @integration (agent) `mise run //packages/kuru-runtime:test` -> observed: exit 0; 225 passed, 0 failed (163.1 s test time)

## 3. Sibling PR #181 merges cleanly

- [x] 3.1 @integration (agent) `git merge --no-commit --no-ff da4033ed` on fix commit `1b3e8712`, then `mise run //packages/kuru-runtime:test -- cancelled_dream_abandons` in the merged tree, then `git merge --abort` -> observed: "Automatic merge went well", no conflicted paths; merged tree compiled and the test passed (1 passed)
- [x] 3.2 @integration (agent) detached `da4033ed`, `git rebase 1b3e8712` -> observed: all four #181 commits replayed, "Successfully rebased"

## 4. Static checks

- [x] 4.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> observed: format, lint, lint:windows and typecheck each exit 0 (run with `NODE_OPTIONS` unset); `validate --all --strict` 0 errors, 0 warnings
