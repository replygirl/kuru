# Proposal

## Why

Main 11af1ff7 went red in CI run 37147327774 (macos-latest coverage partition
3 of 4, job 111273991362): after all 509 tests passed, the partition's
self-check reported "the line export does not reproduce llvm-cov's summary ...
packages/kuru-memory/src/service.rs: llvm-cov 3283/7612, port 3272/7612". The
line export port is not at fault. The partition writes three cargo-llvm-cov
reports (`--lcov`, `--json`, `--json --summary-only`), and cargo-llvm-cov 0.9.1
re-merges every `*.profraw` in the target root for each report
(`src/report.rs`: `generate` calls `merge_profraw`, which globs the target
root). A memory-service stand-in owner, an instrumented child process that its
test left polling, exited at its 120 s backstop between the `--json` and
`--summary-only` reports. compiler-rt writes a profile from its exit hook
(`InstrProfilingFile.c`, `__llvm_profile_register_write_file_atexit`), so the
summary counted stand-in-only `service.rs` lines that the full export never
saw. The port is a pure function of the full export and reproduces llvm-cov
exactly over one profile set; the self-check compared two different sets.

The stand-in outlives its test because the readiness tests run on a paused
clock: the test can finish and remove the fixture directory, including the
release file the stand-in polls for every 10 ms of real time, before the
stand-in's first poll. It then polls a vanished file until its 120 s bound.
Locally, a `service::` run left four such processes alive for 121 s after the
test binary exited.

## What Changes

- The stand-in owner consumes its release (removes the file) immediately
  before it exits. The stand-in fixture writes only its first release and,
  once the stand-in was spawned, waits in real time (30 s bound) until the
  release is consumed before `start` returns, so the stand-in never outlives
  its test. The 120 s bound stays as a backstop.
- The coverage partition records its raw profiles (name, size, modification
  time) once its tests end and requires the same set after its last report,
  before the line-export self-check, and again after its receipt. A changed set
  fails the partition naming the new, changed and removed profiles ("an
  instrumented process outlived the partition's tests") instead of blaming the
  port. Every export and the receipt's profile digest then describe one
  profile set, which the existing requirement already presumes ("over the same
  profiles", docs/development.md).
- The line export port, its self-check against cargo-llvm-cov's
  `--summary-only` figures, and the 90% gate are unchanged. The branch and
  change name record the symptom (the port's self-check); the cause and fix
  are outside the port.

## Capabilities

### New Capabilities

### Modified Capabilities

None. `repository-delivery` requires each partition's line export to
reproduce "the partition's own" cargo-llvm-cov `--summary-only` figures; that
requirement is right and is unchanged. The implementation failed to hold the
partition's profiles fixed across the reports it compares, and a test fixture
left an instrumented process running; this change corrects both.

## Impact

- `packages/kuru-delivery/src/coverage/orchestrate.rs`: `ProfileSet` replaces
  `profile_totals`; `export_lines` and the receipt step require an unchanged
  set; fake-host and unit tests.
- `packages/kuru-memory/src/service.rs`: `stand_in_owner` consumes its
  release; the `progress_wait` fixture waits for consumption; a regression
  test.
- `docs/development.md`: the profile-set check and the stand-in's consumed
  release.
- No dependency, protocol, workflow or gate change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
