# Tasks

## 1. Probe trace

- [ ] 1.1 Add the shared trace prelude to `apps/kuru-tui/tests/windows_cli.rs` and prepend it to the source-entrypoint probe with bracketing marks, and verify the probe's statements and assertions are otherwise unchanged in the diff
- [ ] 1.2 Add `trace_failure` and `launch_traced`, route both source-entrypoint tests through them, and verify the Windows-target lint compiles the file
- [ ] 1.3 Add the Windows-only parked scratch-probe test asserting the timeout error names the parked function and its classification, and verify it compiles under the Windows-target lint

## 2. Verification

- [ ] 2.1 Run `mise run format:check`, `mise run lint`, the Windows-target lint and `mise run typecheck`, and verify each exits 0
- [ ] 2.2 Run the `kuru-delivery` and `kuru-tui` test tasks locally on the host, and verify they pass (the touched file is `cfg(windows)`, so the host run proves only that nothing else regressed)
- [ ] 2.3 (CI-only) Windows native tests: the parked scratch-probe test passes and both source-entrypoint tests pass; record the trace span against the launch wall time from a passing run as the overhead measurement
- [ ] 2.4 (CI-only, next natural occurrence) A probe timeout's panic carries the trace classification and tail; report it to the lead without changing shipped code
