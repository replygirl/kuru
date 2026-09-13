## 1. Fixture isolation

- [x] 1.1 @regression (agent) build ordinary memory, TUI, connector, and platform targets without `test-support` -> `mise run //packages/kuru-memory:build`, `//packages/kuru-platform:build`, `//packages/kuru-connectors:build`, and `//apps/kuru-tui:build` each exited 0 on 2026-09-12 using a verified read-only bundle mirror.
- [x] 1.2 @integration (agent) compile focused behavioral consumers with `test-support` -> `mise run //packages/kuru-runtime:typecheck` and `//apps/kuru-tui:typecheck` each exited 0 on 2026-09-12, compiling their existing real-engine fixture imports.

## 2. Existing prepared-input diagnostic [native]

- [~] 2.1 @runtime (agent) run the isolated missing/corrupt Windows prepared-input build check -> defer: Windows-native `bundle:verify-native-build` remains required; this change does not modify `build.rs` or that pre-existing diagnostic task, and no local Windows result is claimed.
