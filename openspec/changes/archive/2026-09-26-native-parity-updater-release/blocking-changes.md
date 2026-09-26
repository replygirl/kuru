# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

PR stacking note (not a cospec change): this branch stacks on PR #107
(`ci/native-coverage-shards`, archived here as `2026-09-26-native-coverage-shards`),
whose Windows shard and report jobs it keeps. It also depends on PR #108
(`test/previous-release-update-ci`, cospec change `previous-release-update-ci`,
not yet in this tree), which defines
`//packages/kuru-delivery:test:previous-release-update`; this change only
invokes that task and must merge after, or together with, #108. Without #108 every
`install` job and Unix staged leg fails with mise "task not found". PR #106
(`ci/runner-images`) changes the Ubuntu/macOS labels and removes Intel macOS in
the same files; this change touches only the Windows label.
