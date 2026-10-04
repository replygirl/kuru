# Verification

## 1. Concurrent owned spawns keep pipes private [critical]

- [x] 1.1 @regression (agent) macOS: run the forced-window unit test with only the platform spawn lock acquisition disabled (`pipe()` and FIOCLEX kept as two steps; the red test commit precedes the lock commit, so the mutation is that commit pair's diff) -> fails on its first iteration: A's `cat` does not reach EOF within the bound while B holds A's pipe -> MEASURED at test commit 3c4076a0 (no lock), macOS arm64: 20/20 runs failed, each with "A's cat did not reach end of file within 5s while B ran: Err(Timeout) (B's first event: Ok(Ok(Spawned)))"
- [x] 1.2 @regression (agent) macOS: run the same test with the fix, ×20 -> 20/20 pass, B's first event is `Blocked` every time -> MEASURED at fix commit d1c5f17b: 20/20 passed (the test asserts `Blocked`)
- [~] 1.3 @regression (agent) Linux: run the same test with the fix, expecting a pass with B's first event `Blocked` (unfixed Linux would report `Spawned` yet still reach EOF: pipe2 is atomic) -> defer: no local Linux host; runs in the native Linux platform CI job on push (not yet observed)
- [x] 1.4 @unit (agent) macOS: pipe ends created by the platform carry close-on-exec (`platform_pipes_are_close_on_exec`) -> MEASURED: passes in the full platform suite (34 unit tests passed)
- [~] 1.5 @unit (agent) Linux: the same close-on-exec assertion on the `pipe_with(CLOEXEC)` path -> defer: no local Linux host; native Linux platform CI job on push (not yet observed)
- [x] 1.6 @integration (agent) concurrent `cat` sentinel in `tests/unix_process_group.rs`, ×20 -> all pass; each child exits on its own EOF while the other is alive -> MEASURED macOS, load ~30-45: with the fix 20/20 runs passed (50 iterations each); without the lock (3c4076a0) 6/20 runs failed, at iterations 1, 18, 21, 3, 29, 28
- [x] 1.7 @runtime (agent) macOS: rerun `shared_budget_holds_leases_through_owned_reap_and_caps_invocations` with a scratchpad harness equivalent to `diag-connectors-hook-start-watch.sh` (that script hardcodes another session's paths) with 6-8 concurrent loops for at least the 2000 runs that produced 13 hangs before -> zero hangs at the 5 s deadline -> MEASURED, 7 concurrent loops: fixed binary 0 failed of 2002 (load ~20-26; only the 7 cold first runs exceeded 1.5 s, at ~1.95 s) and 0 failed, 0 slow of 2002 (load ~35-48); pre-lock binary (3c4076a0) 14 failed of 2002, each at ~5.0-5.4 s ("hook two/one did not start before its timeout")
- [x] 1.8 @manual (agent) the stall amplification is stated: a pre-existing per-spawn stall on std's fork path becomes process-wide for owned spawns, including the cleanup snapshot; mitigation is migrating the memory supervisor/engine spawns behind the platform, not a timeout -> present in design.md Risks, the `unix.rs` module doc, AGENTS.md and docs/development.md (commits d1c5f17b, 566641fa)

## 2. Owned callers unchanged in behavior

- [x] 2.1 @integration (agent) `mise run //packages/kuru-connectors:test -- -- hooks unix_shell rpc` ×3 -> pass -> MEASURED: exit 0 three times, 39 passed each; full connectors suite exit 0 (302 lib tests passed)
- [x] 2.2 @integration (agent) `mise run //packages/kuru-platform:test` full -> pass -> MEASURED: exit 0 (34 unit, 21 filesystem, 4 local_ipc, 3 unix_process_group, 7 unix_snapshot passed)
- [x] 2.3 @integration (agent) `cargo test -p kuru-delivery --all-features --lib -- coverage::tests` (owned coverage group callers) -> pass -> MEASURED: exit 0, 35 passed including the four `unix_group_*` tests and `live_libtest_lists_and_runs_exact_selections`
- [~] 2.4 @integration (agent) Linux CI native platform and connector jobs pass on the pipe2 path -> defer: CI runs on push; reruns are not permitted for this change; not yet observed

## 3. Static checks and docs

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` -> pass -> MEASURED: each exit 0
- [x] 3.2 @unit (agent) `mise run docs:check` and `mise run cospec -- validate --all --strict` -> pass -> MEASURED: each exit 0
