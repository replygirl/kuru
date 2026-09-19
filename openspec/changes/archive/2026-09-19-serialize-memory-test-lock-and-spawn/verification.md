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

## 4. 2026-09-19 follow-up: gate direct acquire-expecting lock probes

CI run 35425862127 (native-build macos-15-intel, `mise run //packages/kuru-memory:test`)
failed `provision::native_tests::rejected_activation_preserves_verified_stage_and_occupied_destination`
at `contender.try_lock().unwrap()` — a direct acquire-and-expect-success probe
outside the choke-point helpers this change's original inventory covered
(see design.md's 2026-09-19 follow-up correction). Fixed by
`fix/memory-test-spawn-gate` (branch, not re-archived here).

- [x] 4.1 @unit (agent) gate every direct `try_lock()`/`lock()` call across
  `server_tests.rs`, `provision/native_tests.rs`, `provision/tests.rs`,
  `files.rs`, `store/*_tests.rs`, `store.rs`'s inline test module and
  `test_support.rs` whose result is asserted to succeed (`.unwrap()`,
  `.expect(`, `?`, `.is_ok()`) and that was not already under the gate's
  exclusive side -> 4 sites gated in `provision/native_tests.rs`
  (`rejected_activation_preserves_verified_stage_and_occupied_destination`
  plus 3 Windows-only recovery tests), 1 in `store.rs`
  (`private_paths_and_stable_lock_fail_closed`), 1 in `test_support.rs`
  (`snapshots_survive_source_removal_and_replacement_and_reject_corrupt_private_bytes`);
  every other `.lock()`/`try_lock()` in those files is either already gated,
  a genuine `Err(WouldBlock)`-expecting probe (left ungated by design), a
  self-retrying poll loop with its own deadline (tolerant of a spurious
  `WouldBlock` by construction), or an in-memory `std::sync::Mutex`/
  `tokio::sync::Mutex` unrelated to the flock/posix_spawn race
- [x] 4.2 @unit (agent) `mise run format:check` -> passes
- [x] 4.3 @unit (agent) `mise run //packages/kuru-memory:lint` -> passes, 0 warnings
- [x] 4.4 @unit (agent) `mise run typecheck` -> passes, exit code 0
- [x] 4.5 @unit (agent) `cargo test -p kuru-memory --lib --all-features -- provision:: server::`
  run three consecutive times -> 0 failures in all three runs
- [x] 4.6 @unit (agent) `mise run cospec:validate` -> passes
