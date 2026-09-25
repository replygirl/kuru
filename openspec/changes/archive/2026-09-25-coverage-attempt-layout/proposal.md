## Why

Review of the archived `coverage-stall-evidence` change found that the pinned
`actions/download-artifact` v8.0.1 extracts a pattern download that matches one
artifact directly into the destination, so the Windows coverage report misreads
every ordinary single-attempt shard; the report could also validate an older
receipt after a later attempt of the same shard failed, and the relay, script
and record had smaller gaps.

## What Changes

- `packages/kuru-delivery/src/coverage.rs` (delivery CI tooling, not application
  source): `write_receipt` requires a canonical attempt and writes its evidence
  under `<output>/attempt-<n>/`, so each uploaded artifact names its own attempt.
  Collection accepts both download layouts: `<shard>/attempt-<n>/` for one
  artifact and `<shard>/<artifact name>/attempt-<n>/` for several, requiring the
  name and inner attempt to agree and refusing mixed layouts. `relay_output`
  keeps draining and relaying the test's stdout after a log or relay write error
  and reports that error in its outcome. Specific refusals replace two broad
  `is_err()` assertions; the supervision tests run on paused Tokio time.
- `packages/kuru-delivery/support/windows-coverage.ps1`: shard inputs and the
  diagnostics directory are validated and created first, and one `try` covers
  the whole shard so any failure copies manifests, runner ledger and the error
  into diagnostics.
- `.github/workflows/native-tests.yml`: the report job first refuses unless
  `needs.windows-coverage.result` is `success`; comments state the attempt
  layout and download budget.
- `docs/development.md`, `docs/verification.md`: "latest uploaded (successful)
  attempt", the success precondition, the download budget, and the deadline's
  scope (test executables, not compilation); reflowed.
- `packages/kuru-delivery/Cargo.toml`: the workspace Tokio with `test-util` as a
  dev-dependency for paused-time tests, as `kuru-runtime` and `kuru-memory` do
  (no new crate or lockfile change).
- `tests/release_workflow.rs`, `tests/powershell_diagnostics.rs` contract tests.
- `openspec/changes/archive/2026-09-25-coverage-stall-evidence/tasks.md` and
  `verification.md`: record plainly that its artifacts and apply gate followed
  the implementation (a text correction, not a directory move).

## Impact

`windows-coverage` shard jobs (artifact contents gain an `attempt-<n>/` level)
and `windows-coverage-report` (new first step, layout handling). The 90% gate,
receipt identity checks, job limits, `native-gate` inputs and secrets are
unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
