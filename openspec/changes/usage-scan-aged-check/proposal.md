# Proposal

## Why

The usage-ledger startup scan was quadratic in ledger size until
`usage-scan-index-range` made it linear, and nothing in CI would notice a
regression. A gating Ubuntu job now measures the scan from the owner open
timeline on aged stores at 1,000 and 5,000 conversations and fails when it
grows faster than linear or passes an absolute ceiling.

## What Changes

- `.github/workflows/ci.yml`: a new `usage-scan-scaling` job ("Usage scan
  scaling (ubuntu-latest, provisional bounds)"), listed in `ci-gate`'s `needs`.
  It imports the run's verified bundle inputs into a runner-temporary private
  bundle directory, builds the release kuru-memory test-support tooling,
  restores the aged fixture by exact key only (saved from `main` only; on a miss
  it ages the stores in-job and never skips), then runs the gated measurement
  with `--assert`. It uploads the records and timelines as
  `ci-usage-scan-attempt-<n>`. The actions are pinned by the SHAs the file
  already uses. `KURU_MBX=0` comes from the workflow env. There is no new
  workflow file, no retry and no `continue-on-error`.
- `packages/kuru-memory/src/test_support/usage_scan.rs` (new, behind the
  package's `test-support` feature, never compiled into `kuru`): the fixture
  key, which is composed from compiled constants so the YAML never repeats
  them; store creation; sealing; the measurement driver; timeline parsing; and
  the evaluation of the provisional bounds. `src/main.rs` adds the
  `usage-scan-fixture` and `measure-usage-scan` subcommands. `test_support.rs`
  declares the module.
- Visibility only, no behaviour: `store::migrations::{CURRENT_VERSION,
  USAGE_CURRENT_VERSION}` and `EndpointRecord::directory` become `pub(crate)`,
  and `aged_store::REPORT_FORMAT_VERSION` becomes `pub`, so the key and the
  driver read the real values instead of copies.
- `packages/kuru-memory/mise.toml`: `measure:age-store` gains
  `--profile <profile>` (default `dev`). The new tasks are
  `measure:usage-scan:fixture` and `measure:usage-scan`; only the second sets
  `KURU_OPEN_TIMELINE=1`.
- `packages/kuru-delivery/tests/release_workflow.rs`: the `ci-gate` needs
  string, the `native-platform` job slice, and a test pinning the new job's
  shape.
- `docs/development.md`: the job, its provisional bounds, the fallback decision
  rule, the fixture key, local use and timeline leftovers.

## Impact

- **One new required job** under `ci-gate`, on ubuntu-latest, with
  `timeout-minutes: 45` like the other native jobs.
- **One new Actions cache key family,** `usage-scan-fixture-v1-<sha256>`,
  saved only on `main`. It holds two aged stores; its size is recorded on the
  first run.
- **No secrets, and no product behaviour change.**
- **Bounds.** The bounds are provisional: ratio K = 8, a 100 ms floor and a
  3 s ceiling at 5k. Calibrated bounds and their derivation land with the
  validation-record change, which also adds the zero-rows-decoded assertion on
  a recorded reopen.
- **Row counts.** A driver-created store holds exactly 4,000 and 20,000 owned
  usage rows, not the design's 4,550 and 20,500. So N2/N1 = 5. Linear growth
  predicts a ratio of about 5 and quadratic about 25, and K = 8 still separates
  the two.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
