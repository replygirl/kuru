## Why

Native coverage is orchestrated by a Windows-only PowerShell script, while
Linux and macOS run one monolithic job whose test executables are launched by a
bare `Command::status()`, so the documented deadline, stall report and owned
cleanup hold only on Windows. One Rust orchestrator on every OS gives all three
platforms the same five fail-closed shards, supervised deadline and single 90%
report, with no product behavior change.

## What Changes

- `packages/kuru-delivery/src/coverage.rs` and new
  `packages/kuru-delivery/src/coverage/orchestrate.rs` (CI coverage tooling
  behind the `tooling` feature, not shipped in the `kuru` executable): the
  shard workspace package list is derived from `SHARDS`; collect takes an
  `artifact_os` label for its artifact marker; Unix test executables are
  supervised through `kuru_platform::unix::OwnedProcessGroup` with the same
  deadline, stall report and signal-before-reap cleanup as Windows; the
  shard/collect orchestration formerly in PowerShell (fresh target and
  diagnostics, `cargo-llvm-cov show-env` parsing, inventory, selection, runner
  configuration, receipt, report) moves to Rust behind an injectable
  cargo/process boundary.
- `packages/kuru-delivery/src/main.rs`: `kuru-delivery coverage shard` and
  `coverage collect` subcommands.
- `packages/kuru-delivery/mise.toml`: OS-agnostic `coverage:shard` and
  `coverage:collect` replace `coverage:windows:shard`/`:collect`;
  `packages/kuru-delivery/support/windows-coverage.ps1` is deleted.
- `.github/workflows/native-tests.yml`: every OS runs the five `SHARDS` as a
  matrix and one collect job enforcing `--fail-under-lines 90` once; the Unix
  monolithic `coverage` job is removed; the per-OS install job is kept;
  `CARGO_PROFILE_TEST_DEBUG` (Linux `0`, otherwise `line-tables-only`) is
  identical in shard and collect; one rust-cache key per OS saved by one shard;
  bundle directories under `runner.temp`; `KURU_MBX=0`.
- `packages/kuru-delivery/tests/release_workflow.rs` and
  `tests/powershell_diagnostics.rs`: workflow-text and launch assertions follow
  the new jobs and tasks.
- `docs/development.md`: the per-OS shard topology, Unix process-group
  termination, artifact names, the new tasks and a local orchestrator recipe.

## Impact

Jobs: `native-tests` on Linux and macOS gains five shard jobs and a report job
and loses the monolithic `coverage` job; Windows keeps the same shard/report
shape under renamed jobs and artifacts `<prefix>-coverage-<os>-<shard>-…`.
Shard `timeout-minutes` and `KURU_COVERAGE_JOB_MINUTES` stay equal. The 90%
workspace line gate, receipt identity checks, secrets (none) and required checks
(`ci-gate`, `Lint PR title`) are unchanged; `native-gate` topology changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
