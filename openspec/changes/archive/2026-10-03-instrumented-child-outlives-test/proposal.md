# Proposal

## Why

PR #133's CI run 37164260608 failed ubuntu-latest coverage partition 8 after all
271 tests passed: the raw-profile invariant from #200 found
`kuru-13664-10634524661574408682_0.profraw` written while the partition exported
its coverage. The last test of the last executable,
`kuru_runtime::tests::provider_free_undo_preserves_sessions_and_archives_added_identities`,
drops its temporary memory store without `close()`. The dropped store hands its
Dolt supervisor, which leads its own process group, to a detached reaper
thread; the supervisor stops Dolt and exits on its own about 0.25 s later, after
the test process exited, and writes its profile during the exports.

The failure could not say which binary or test wrote the profile: a profile
name carries a process ID and a per-executable module signature, the runner
ledger records only profile counts, and no artifact listed profile names.

## What Changes

- The leaking test closes its store before it returns, and proves it with a new
  test-support check: `test_support::supervisor_mark()` before it opens memory,
  `test_support::unawaited_supervisors(&mark)` before it returns, which lists
  every Dolt supervisor the test started that is unreaped or was dropped
  without a close.
- Test support appends a spawn row (`"record": "spawn"`: pid, parent, role,
  executable, originating test) to the runner ledger for every instrumented
  child that leaves the test's process group: memory Dolt supervisors and
  service owners (with the ledger forwarded to an owner's own spawns) and
  pseudo-terminal children. The coverage runner names the ledger to each test
  process (`KURU_COVERAGE_SPAWN_LEDGER`) and adds a row for each test
  executable's listing profile.
- The partition's changed-profile failure names each profile's writer from those
  rows: "pid N is the `<role>` `<executable>` started by test `<name>`", and the
  executables that share its module signature. Spawn rows never enter a plan.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory`: `test_support::spawn_ledger` (new),
  `test_support::{SupervisorMark, supervisor_mark, unawaited_supervisors}`, the
  engine ledger's dropped-owner record, and test-support-only rows at the
  supervisor and owner spawn sites in `server.rs` and `service.rs`.
- `packages/kuru-runtime/src/tests.rs`: the leaking test.
- `apps/kuru-tui/tests/support/terminal.rs`: terminal children's rows.
- `packages/kuru-delivery/src/coverage`: `spawns` (new), spawn-row parsing in
  `plan::read_ledger`/`read_spawns`, the runner's environment and listing rows,
  and the attributed invariant message.
- `docs/development.md`: coverage partition section.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
