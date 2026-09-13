## 1. Build hygiene

- [x] 1.1 Gate memory fixture exports and fixture binary behind `test-support`, and remove test-support features from ordinary build/release tasks. Observed: ordinary memory, platform, connector, and TUI package build tasks exit 0 without `--all-features`.
- [x] 1.2 Enable memory test support for existing behavioral consumers and verify focused test targets still compile. Observed: runtime and TUI all-target typechecks exit 0 with their real test-support imports.
