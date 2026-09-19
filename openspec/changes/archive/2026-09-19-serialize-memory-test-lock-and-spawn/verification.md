## 1. The target flake is removed at its cause [critical]

- [x] 1.1 @regression (agent) run `cargo test -p kuru-memory --all-targets --all-features --locked -- server::tests::supervisor_rejects_bad_configuration_and_parent_eof_without_spawning` (the exact test named in the diagnosis) inside the full `mise run //packages/kuru-memory:test` suite, so it runs concurrently with the real-Dolt fixtures that supplied the original race -> observed passing: `mise run //packages/kuru-memory:test` (real bundled engine) — lib binary `test result: ok. 155 passed; 0 failed; ...; finished in 186.12s`, including `server::tests::supervisor_rejects_bad_configuration_and_parent_eof_without_spawning ... ok`; whole task `Finished in 262.85s`, exit code 0, 179 tests total across every binary, 0 failed
- [x] 1.2 @regression (agent) rerun the `server::` and `provision::` subsets of `cargo test -p kuru-memory --lib` at least twice more each, back to back, to raise confidence the race window stays closed under repeated concurrent scheduling -> observed passing every time: `-- server::` 18/18 three consecutive times (3.71s, 3.67s, 3.66s); `-- provision::` 30/30 three consecutive times (7.31s, 7.66s, 7.72s); 0 failures in any of the six runs
- [x] 1.3 @unit (agent) run `spawn_gate::tests::an_in_flight_spawn_excludes_lock_acquisition_until_it_finishes` and `spawn_gate::tests::a_panicking_holder_releases_the_gate` -> both pass deterministically (no wall-clock wait in either): both listed `... ok` in the full-suite lib run above

## 2. No behavior, assertion or product code changed

- [x] 2.1 @unit (agent) run the full `store::`, `provision::` and `server::` suites (the ones touched) end to end -> every test passes with its original assertions (no assertion text loosened, no `#[ignore]` added, no new `unwrap_or`/timeout widening): covered by the 155/155 lib-binary pass above (git diff review confirms only `{error:#}` diagnostics were added to existing assertions, never a condition weakened)
- [x] 2.2 @unit (agent) `cargo build -p kuru-memory --release` (or equivalent non-test check) -> no `spawn_gate` symbol reachable; the gate is `#[cfg(test)]`-only and adds nothing to a shipped binary: `mise run //packages/kuru-memory:build` (non-test, default features) succeeded; `nm -g target/debug/libkuru_memory.rlib | grep -iE "spawn_gate|gated_cache_lock|spawn_gated_open"` -> 0 matches

## 3. Static checks stay green

- [x] 3.1 @unit (agent) `mise run format:check` -> passes (root aggregate, including `//:format:rust`)
- [x] 3.2 @unit (agent) `mise run //packages/kuru-memory:lint` (clippy `-D warnings`, `--all-features`) -> passes, 0 warnings
- [x] 3.3 @unit (agent) `mise run typecheck` (workspace, including `apps/kuru-tui` which depends on `kuru-memory`) -> passes, exit code 0
