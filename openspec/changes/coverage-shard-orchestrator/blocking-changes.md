# Dependencies

## Blocked by

- [x] `native-coverage-shards` — five `SHARDS` rows (`memory` and `runtime` separate), the shared per-OS coverage cache key and the matrix-equals-`SHARDS` workflow test (PR #107) *(archived 2026-09-26)*

## Soft-blocked by

None.

PR stacking note (not machine-checked): this change also depends on PR5's
cospec change `native-parity-updater-release` (the per-OS `install` job this
change keeps, `runner.os` step conditionals and the Windows `-latest` label).
That change exists only on PR5's unpushed branch, so `cospec validate` rejects
it here as a dangling reference. This branch is cut from PR #107
(`ci/native-coverage-shards`) and rebases onto PR5 before task group 5, whose
workflow edits assume PR5's jobs and labels; add the entry above once PR5's
change is present in this tree.
