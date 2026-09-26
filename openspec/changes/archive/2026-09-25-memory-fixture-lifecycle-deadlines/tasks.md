## 1. Shared lifecycle deadline

- [x] 1.1 Move the facade helper into `test_support` as `fixture_deadline(n)`, add the test-only server `close_budget()` retirement term, and verify the facade tests still compile and pass unchanged in lifecycle counts.

## 2. Remaining outer lifecycle backstops

- [x] 2.1 Replace the flat outer backstops around real lifecycles in `packages/kuru-memory/src/service.rs` and `packages/kuru-memory/src/store.rs` with `fixture_deadline(n)` (plus `SERVICE_IDLE_TIMEOUT` where the owner retires only through idle expiry), and verify by review that each count matches the fixture body and that inner waits and assertions are unchanged.
- [x] 2.2 Run the affected service, store and facade tests through `mise run //packages/kuru-memory:test`, check the Windows-only sites against the Windows target, and record the observed results, naming any check that could not be run.

## Sites

`fixture_deadline(n)` = `LOCK_TIMEOUT` (180s) + n × (`startup_timeout_secs` 30s + `QUERY_TIMEOUT` 30s + max(30s permit deadline, 32s `close_budget()`)) = 180s + n × 92s. Facade sites keep their counts (n = 1/2/3/6 → 272/364/456/732s).

`service.rs`:

| Fixture | n | Idle (+30s) | Old | New |
| --- | --- | --- | --- | --- |
| maintenance retires idle owner | 1 | | 90s | 272s |
| retiring endpoint handshake | 1 | | 90s | 272s |
| Windows starter job exit | 1 | yes | 110s | 302s |
| Windows denying outer job (`timeout_at`) | 2 | | 110s | 364s |
| owner reaps real Dolt | 1 | | 90s | 272s |
| lost reply receipt and owner restart | 2 | | 90s | 364s |
| crashed owner receipt (`timeout_at`) | 2 | | 120s | 364s |
| lost usage reply and restart | 2 | | 90s | 364s |
| candidate transition query | 2 | | 90s | 364s |
| lost candidate transition replies | 2 | | 90s | 364s |
| lost candidate begin reply | 4 | | 90s | 548s |
| independent clients elect one process | 1 | yes | 120s | 302s |
| rejected publication reaps engine | 2 | | 90s | 364s |
| idle accept deadlines | 1 | | 90s | 272s |
| separate cold starters (Unix) | 1 | yes | 100s | 302s |

`store.rs`:

| Fixture | n | Old | New |
| --- | --- | --- | --- |
| service disconnect and owner restart | 4 | 90s | 548s |
| committed promotion cleanup retry | 1 | 90s | 272s |
| candidate transition observation | 1 | 90s | 272s |
| selected candidate inventory | 1 | 90s | 272s |
| exact candidate outcome across restart | 2 | 90s | 364s |

These were considered and left unchanged because they are not outer backstops around a whole lifecycle:

- `service.rs` in-flight outcome 40s: it starts after the owner is open.
- `service.rs` cancelled call 10s: it runs against a fixture server.
- `provision/native_tests.rs` 30s: it is the assertion that warm opens do not wait.
- `test_support/windows.rs` 40s: it waits for an observer command.
- The inner cold-start waits are reported, not changed: `service.rs` Windows starter readiness 40s, Unix cold-starter output 60s, and `tests/windows_lifecycle.rs` ready-marker observations 60s and 30s. The integration test cannot reach the crate-private helper.

## Observed evidence

Local macOS aarch64 run, 2026-09-25, on a shared machine:

- `mise run //packages/kuru-memory:test -- --lib -- service::tests facade::tests store::tests::{the five changed store fixtures}` (prefetch, `--all-features`, prepared supervisor): 48 passed, 0 failed (162.79s). This includes every changed Unix-visible fixture: all 17 facade fixtures, the 13 non-Windows `service.rs` fixtures and the 5 `store.rs` fixtures.
- `mise run //packages/kuru-memory:lint` (clippy `-D warnings`) passed, and `cargo fmt --check` is clean.
- Not run: the two `#[cfg(windows)]` `service.rs` fixtures. `cargo check --target x86_64-pc-windows-msvc` cannot build on this host because `libsqlite3-sys` needs the Windows C headers. Those edits only change the deadline expression, using names already in scope in those functions. Native Windows CI on this commit is the gate.
