# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

PR #181 (`fix/runtime-cancelled-dream-join-wait`, head `da4033ed`) rewrites
the middle of this same test: a `DreamWatch` that owns the harness event
receiver and lists `marker` and `survived` as report files, event-driven
`progress_wait` waits for the marker and the join, and new helpers after the
test. This change therefore edits only the two regions #181 leaves alone: the
hook script string (the `marker`/`survived` bindings and format arguments are
unchanged) and the tail after the `kuru-hook` message assertion. It adds no
event subscriber and no read between the marker wait and the join.

Measured 2026-10-03 against fix commit `1b3e8712` (test file identical to this
change's final tip): `git merge --no-commit --no-ff da4033ed` reported
"Automatic merge went well" with no conflicted paths, and the merged tree
compiled and passed this test
(`mise run //packages/kuru-runtime:test -- cancelled_dream_abandons`, 1
passed); `git rebase 1b3e8712` of #181's four commits from `da4033ed` also
completed without conflicts. Either landing order is expected to need no
hand resolution of this test.
