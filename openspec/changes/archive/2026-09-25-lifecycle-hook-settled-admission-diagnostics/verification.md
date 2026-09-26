## Evidence environment

Local evidence comes from macOS arm64 on 2026-09-25, in the #89 worktree with its own `target/` (`CARGO_TARGET_DIR` unset, `CARGO_BUILD_JOBS=4`), `RUST_TEST_THREADS=2` and `KURU_TEST_SUPERVISOR_PREPARED=1`. Other agents were compiling on the machine (load average about 6).

Static checks:
- `cargo fmt --all --check` is clean.
- `cargo clippy -p kuru-runtime -p kuru --all-targets --all-features --locked -- -D warnings` is clean.
- `mise run docs:check` passed.

Deviation: the code was drafted before these artifacts were written. `validate --strict` and the apply gate then ran before the recorded evidence and the commit.

## 1. Settled admissions reach the diagnostics ring [critical]

- [x] 1.1 @regression (agent) `unix_shell_turn::debug_cli_rotates_the_fixed_private_diagnostic_ring` on unmodified e167d96b -> failed 3/3 isolated runs with `Condition failed: logs.contains("\"status\":\"error\"")`, matching the pre-push coverage failure. A temporary, reverted ring dump from each failing run held 536 records: 504 `sse.rs` stream-reconciliation records for the 768-call response, then provider, actor and turn completions, and no `kuru.tool` record. With the fix -> passed 10/10 isolated runs, and the stricter assertions (an `admission` record with `error` status, and no `fixture_unknown_tool` anywhere in the ring) hold.
- [x] 1.2 @equivalence (agent) the same rotation test on `fix/restore-green-main` (434d4bad, no pre-dispatch admission) -> passed 10/10 isolated runs; its unoffered calls reached the ToolHost and left `external tool finished` error records.
- [x] 1.3 @integration (agent) full `cargo test -p kuru --all-features --locked --test unix_shell_turn` with the fix -> 5/5 passed, including the normal-ring and debug-equivalence shell turn tests.

## 2. Runtime behavior unchanged

- [x] 2.1 @integration (agent) `cargo test -p kuru-runtime --lib --all-features --locked` -> 220/220 passed (660.6 s under load)

## 3. Native platforms

- [~] 3.1 @integration (agent) native Windows and Linux suites and instrumented coverage -> defer: not pushed, per instruction
- [~] 3.2 @integration (agent) a CLI fixture with a configured denying `pre_tool` hook -> defer: the hook-denial and deliberation sites share `trace_settled_admission` with the speaking unoffered-call site exercised in 1.1; no ring-level hook fixture exists yet.

## Notes

- **Fragility on main (not changed here).** The rotation test's error records survive rotation only because tool settlement is logged after the 768-record stream-reconciliation burst of the same response. That ordering holds on main and on this branch; a future change that adds enough records after tool settlement could rotate them out.
- **Unchanged scope.** Dream proposals and budget-exhausted calls had no tool record before `lifecycle-hook-authority-cleanup` and still have none.
