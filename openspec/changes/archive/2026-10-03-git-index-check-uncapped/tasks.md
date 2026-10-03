# Tasks

## 1. Regression test

- [x] 1.1 Add the cross-platform `kuru-slow-git-fixture` binary and `discovered_local_config_waits_for_a_slow_git_index_check`, and verify it fails on the unfixed code with the timeout message

## 2. Fix

- [x] 2.1 Await the Git index check's exit without the 5 s cap, keeping the spawn-failure message, and verify the regression test and its two sibling discovered-local tests pass

## 3. Checks

- [x] 3.1 Run format:check, lint, lint:windows, typecheck, docs:check and cospec validate --all --strict, and verify each exits 0
