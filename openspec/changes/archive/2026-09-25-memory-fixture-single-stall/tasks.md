## 1. Single-stall fixture deadline

- [x] 1.1 Change `fixture_deadline(fresh, reopened)` in `packages/kuru-memory/src/test_support.rs` to server starts at `server_start_budget()` plus one single-stall term plus one fixture `QUERY_TIMEOUT`, update its doc comment, keep `fresh_open_budget()` for the crashed-owner wait, and verify the defaults give (1,0)=190s, (1,1)=222s, (2,1)=350s and (2,4)=446s.
- [x] 1.2 Bound `warm_runtime_cache()` by `LOCK_TIMEOUT + VERSION_TIMEOUT` plus a named margin, and verify the warm-up test passes warm and against an empty cache.

## 2. Evidence

- [x] 2.1 Re-run the focused memory fixture tests through the package mise test task, run lint, and record the results, naming the Windows-only checks that could not run here.

## Model (default budgets)

- **Formula:** `fixture_deadline(fresh, reopened)` = (4 × fresh + reopened) × 32s server start + 32s single stall + 30s fixture `QUERY_TIMEOUT`. The single stall is max(32s close, 30s query, 30s startup).
- **Values:** (1,0) = 190s, (1,1) = 222s, (1,3) = 286s, (2,1) = 350s, (2,4) = 446s. Idle-retiring owners add 30s, so the largest derived bound is 446s.
- **Nextest:** `terminate-after` must sit above 446s once adopted.
- **Crashed-owner readiness:** it keeps `fresh_open_budget()` (409s with the margin). Its existing cap at the fixture deadline minus 2s now sets it at 220s under the (1,1) backstop.
- **Warm-up bound:** `LOCK_TIMEOUT` 180s + `VERSION_TIMEOUT` 15s + `WARM_UP_MARGIN` 5s = 200s.

## Observed evidence

Local macOS aarch64 run, 2026-09-25:

- `mise run //packages/kuru-memory:test -- -- service::tests facade::tests test_support:: <five changed store fixtures> prepared_snapshot`: the lib tests passed 53 of 53 (159.25s) and `supervisor_snapshot` passed 1 of 1. This includes `single_stall_defaults_match_the_reviewed_bounds`, which asserts 190s, 222s, 350s and 446s.
- Against an empty `KURU_DOLT_CACHE`: the warm-up test and the bounds test both passed (3.28s including the cold install). The `server_lifecycle` integration binary, which uses the warm-up, passed 12 of 12.
- `mise run //packages/kuru-memory:lint` (clippy `-D warnings`) and `cargo fmt --check` are clean.
- Not run: the Windows-only code. It cannot be built on this host because `libsqlite3-sys` has no Windows headers. Native Windows CI is the gate.
