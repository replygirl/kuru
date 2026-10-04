# Verification

## 1. Concurrent owned spawns keep pipes private [critical]

- [ ] 1.1 @regression (agent) macOS: run the forced-window unit test with the platform spawn lock disabled (local mutation, not committed) -> fails on its first iteration: A's `cat` does not reach EOF within the bound while B holds A's pipe
- [ ] 1.2 @regression (agent) macOS and Linux: run the same test with the fix, ×20 -> 20/20 pass, B's first event is `Blocked` every time (unfixed Linux would report `Spawned` yet still reach EOF: pipe2 is atomic)
- [ ] 1.3 @unit (agent) every Unix: pipe ends created by the platform carry close-on-exec -> assertion passes on macOS and Linux
- [ ] 1.4 @integration (agent) concurrent `cat` sentinel in `tests/unix_process_group.rs`, ×20 -> all pass; each child exits on its own EOF while the other is alive
- [ ] 1.5 @runtime (agent) macOS: rerun `shared_budget_holds_leases_through_owned_reap_and_caps_invocations` through `diag-connectors-hook-start-watch.sh` with 6-8 concurrent loops for at least the 2000 runs that produced 13 hangs before -> zero hangs at the 5 s deadline

## 2. Owned callers unchanged in behavior

- [ ] 2.1 @integration (agent) `mise run //packages/kuru-connectors:test` hooks, unix_shell and rpc filters ×3 -> pass
- [ ] 2.2 @integration (agent) `mise run //packages/kuru-platform:test` full -> pass
- [ ] 2.3 @integration (agent) Linux CI native platform and connector jobs -> pass (pipe2 path)

## 3. Static checks and docs

- [ ] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` -> pass
- [ ] 3.2 @unit (agent) `mise run docs:check` and `mise run cospec -- validate --all --strict` -> pass
