## Why

A stalled Windows coverage shard is cancelled at the 90-minute job limit with no
evidence, because the runner's own per-executable wait equals that limit, and its
missing artifact then fails the report. Rerunning only the failed shard also
cannot recover the report, which accepts only artifacts from the current attempt.

## What Changes

- `packages/kuru-delivery/src/coverage.rs` (delivery CI tooling, not application
  source): the Cargo runner stops at a deadline derived from the job start and
  `timeout-minutes`, less a fixed evidence reserve; it relays test stdout while
  keeping a bounded per-executable log, and at the deadline samples, terminates
  and awaits the owned Job tree and writes a stall report naming the tests
  libtest reported as unfinished. Later executables refuse to start.
- `coverage.rs` collection takes each shard's latest attempt no later than the
  current run attempt, validates it exactly (receipt shard and attempt match the
  artifact name; source, tree, toolchain, inventory, ledger and profiles as
  before) and never falls back to an older attempt.
- `packages/kuru-delivery/src/main.rs`: runner-config gains `--diagnostics`,
  `--job-started`, `--job-minutes`; dispatch gains `--diagnostics`, `--deadline`;
  collect's `--run-attempt` becomes `--max-attempt`.
- `packages/kuru-delivery/support/windows-coverage.ps1`: passes the deadline
  inputs and copies manifests and the runner ledger into diagnostics on failure.
- `.github/workflows/native-tests.yml`: records the job start first; uploads
  receipts only on success and diagnostics on failure under a distinct name;
  the report downloads each shard by attempt pattern.
- Tests in `coverage.rs`, `tests/release_workflow.rs` and
  `tests/powershell_diagnostics.rs`; `docs/development.md` and
  `docs/verification.md`.

## Impact

Windows `windows-coverage` shard jobs and `windows-coverage-report`. The job limit
(90 minutes), the 90% gate, receipt identity checks and `native-gate`/`ci-gate`
requirements are unchanged. No secrets. The hung memory-runtime test itself is
out of scope; this change makes it fail inside the job with its name.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
