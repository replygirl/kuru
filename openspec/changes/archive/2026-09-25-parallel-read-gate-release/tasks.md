## 1. Parallel read fixtures

- [x] 1.1 Hoist `ReleaseGateOnDrop` in `packages/kuru-runtime/src/tests.rs` and hold it in every `ParallelReadTestGate` fixture, and verify that with the web release deliberately withheld the fixture reports FAILED within its bounds instead of hanging
- [x] 1.2 Start the web fetch overlap server's bounds at the header-read and response-write steps instead of before managed memory startup, and verify the four gated fixtures pass on the local host
- [x] 1.3 Run strict Cospec validation and Rust formatting, and record that exact-head native Windows CI has not run and is required before merge
- [x] 1.4 Derive every gated fixture's checked-execution entry wait from the temporary store's configured memory startup budget instead of a ten-second literal, name the web overlap request-signal step, correct the #87 and first-poll wording in the proposal, and rerun the four gated fixtures on the local host

## Observed evidence for 1.4

Branch `fix/restore-green-main`, local macOS (aarch64-apple-darwin), 2026-09-25:

- `mise run //packages/kuru-runtime:test -- --lib parallel`: 4 passed (includes `cancelled_parallel_wave_drains_every_owned_read_before_returning` and `refused_parallel_read_does_not_replay_a_later_accepted_serial_effect`).
- `mise run //packages/kuru-runtime:test -- --lib overlap`: 2 passed (`authorized_native_reads_overlap_but_feed_results_back_in_provider_order`, `authorized_web_fetch_overlaps_checked_search_and_keeps_result_order`).
- `cargo fmt -p kuru-runtime --check` and `mise run //packages/kuru-runtime:lint`: clean.
- Not run: native Windows CI on this exact head (required before merge), the full runtime suite and combined coverage.
