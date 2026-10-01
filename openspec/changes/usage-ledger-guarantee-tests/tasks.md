# Tasks

## 1. Owned-row refusal tests (`packages/kuru-memory/src/store/usage_ledger.rs`)

- [x] 1.1 Add a seeding helper that writes one scripted session through the ledger's real write path, and a planting helper that writes a foreign commit on the usage branch through a released server, and verify both by a final writable reopen that reads the seeded usage back
  - `seeded()` writes `mark_new_session`, `admit`, `observe` (seq 1, terminal) and `settle`. It reads the four owned rows and the usage head back.
  - `plant()` first resets the usage branch with `DOLT_RESET --hard` to the seeded head. It then commits the rows with raw SQL and requires `dolt_status` to be 0.
  - Each refusal test ends with `plant(&[])`. That reset is followed by a writable reopen that folds `invocation_count == 1` and `input_tokens == Some(5)`.
- [x] 1.2 Add the malformed-value test (not JSON, unknown field, failed field validation, for each of the four classes at canonical keys) and verify that every case refuses with its decoder's message while a read-only open succeeds
  - `malformed_owned_values_refuse_writable_reopen`: 12 cases, all refused with their exact messages. A read-only open was asserted in each case.
- [x] 1.3 Add the non-canonical-key test (a real value of each class under a key not derived from it) and verify that every case refuses with its key check's message while a read-only open succeeds
  - `owned_values_under_noncanonical_keys_refuse_writable_reopen`: 5 cases, including a record under an uppercase digest.
- [x] 1.4 Add the foreign-key test (unknown class, bare prefix, non-UTF-8 key under the prefix) and verify the unrecognized-key and non-UTF-8 messages while a read-only open succeeds
  - `foreign_keys_under_the_owned_prefix_refuse_writable_reopen` covers `future/x`, the bare prefix, `record` without its slash, and `\xff`.
- [x] 1.5 Add the boundary-key test (`kuru.usage.v1`, `kuru.usage.v1.x`, `kuru.usage.v10x` not refused; `kuru.usage.v1/` refused) and verify the seeded usage still reads back
  - `keys_beside_the_owned_prefix_are_not_refused`. The boundary rows hold values that are not JSON.
- [x] 1.6 Add the pool-not-published test (direct `establish` over a planted row) and verify that the error is the scan's and that `usage_pool` stays unpublished
  - `establish_publishes_no_usage_pool_over_an_invalid_owned_row`.

## 2. Evidence

- [x] 2.1 Stub the owned-state scan out locally (an early `return Ok(())` before its loop, which disables it at both call sites), run the new tests, record which fail and how, revert, and verify that the diff touches only the tests module
  - **Measured on macOS arm64, Dolt 2.3.5.** The stub was `if std::hint::black_box(true) { return Ok(()); }` before `let mut after = None;` in `validate_branch`.
  - `store::usage_ledger::tests`: 5 failed and 10 passed. All 5 new tests failed, with "…the writable reopen activated an invalid usage ledger" or "establish activated an invalid usage ledger".
  - All 10 existing tests passed with the scan removed, so nothing pinned the guarantee before this change.
  - After the revert, the diff is one hunk inside `mod tests`.
- [x] 2.2 Run `mise run //packages/kuru-memory:test`, `mise run format:check`, `mise run lint` (host and Windows targets) and `mise run typecheck`, and record the results with the new tests' added time
  - **Measured on macOS arm64.**
  - `//packages/kuru-memory:test`: exit 0, with all targets green:
    - lib: 531 passed, 4 ignored (602 s);
    - `bundle_build`: 10 passed;
    - `memory`: 5 passed;
    - `server_lifecycle`: 12 passed;
    - `supervisor_snapshot`: 1 passed.
  - `format:check`: exit 0.
  - `lint`: exit 0.
  - `//packages/kuru-memory:lint:windows` (x86_64-pc-windows-msvc clippy): exit 0.
  - `typecheck`: exit 0.
  - **Run alone, the new tests took:**
    - malformed values: 19.1 s;
    - non-canonical keys: 8.4 s;
    - foreign keys: 7.2 s;
    - boundary keys: 4.0 s;
    - pool not published: 0.7 s.
  - **Not run:** Linux and native Windows. These are left to CI; no CI job was rerun.
