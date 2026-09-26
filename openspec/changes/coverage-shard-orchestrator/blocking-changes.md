# Dependencies

## Blocked by

- [x] `native-coverage-shards` — five `SHARDS` rows (`memory` and `runtime` separate), the shared per-OS coverage cache key and the matrix-equals-`SHARDS` workflow test (PR #107) *(archived 2026-09-26)*

## Soft-blocked by

None.

PR stacking note (not machine-checked): this change is also blocked by PR5's
cospec change `native-parity-updater-release` (the per-OS `install` job this
workflow carries verbatim, `runner.os` step conditionals and the Windows
`-latest` label). It exists only on PR5's unpushed branch
(`ci/native-parity-updater`), so `cospec validate` rejects it here as a
dangling reference. Add it as an unchecked "Blocked by" entry after rebasing
onto PR5.
