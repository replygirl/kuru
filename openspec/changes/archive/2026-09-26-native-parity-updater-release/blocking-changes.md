# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

PR stacking note (not a cospec change): this branch stacks on PR #107
(`ci/native-coverage-shards`, archived here as `2026-09-26-native-coverage-shards`),
whose Windows shard and report jobs it keeps. It is based on PR #108
(`test/previous-release-update-ci`, cospec change `previous-release-update-ci`),
which is merged and defines
`//packages/kuru-delivery:test:previous-release-update`; this change only
invokes that task. PR #106
(`ci/runner-images`) changes the Ubuntu/macOS labels and removes Intel macOS in
the same files; this change changes only the Windows label, and its Linux arm64
staged leg reuses the build matrix's existing `ubuntu-24.04-arm` label.
