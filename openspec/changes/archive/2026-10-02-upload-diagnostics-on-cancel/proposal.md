# Proposal

## Why

When a native-test partition stalls until the job's time limit or a host cancel
ends it (main run 36982408751 job 110759990502, Windows on Arm partition 3,
cancelled at 55 minutes with the test step still in progress and no log blob;
jobs 109506600180 and 110722395831 had the same shape), the "Upload failed
partition diagnostics" step is skipped because it runs only on `failure()`, so
the evidence of the stalled partition is lost.

## What Changes

- `.github/workflows/native-tests.yml`: the "Upload failed partition
  diagnostics" step runs on `failure() || cancelled()`.
- `.github/workflows/ci.yml`: the same step in the Ubuntu on Arm memory
  partitions gets the same condition.
- `.github/workflows/bundle-build.yml`: "Upload both builds for diagnosis"
  changes from `failure()` to `failure() || cancelled()`.
- `packages/kuru-delivery/tests/release_workflow.rs`: the workflow guard that
  pins the diagnostics condition expects the new condition (the only
  non-workflow edit; it asserts workflow text and changes no shipped behavior).
- Unchanged by design: the success-only receipt evidence uploads, the
  already-`always()` usage-scan evidence upload, the `!cancelled()` merge-report
  uploads, every cache save, and every release step. No other workflow has a
  `failure()`-gated diagnostics or evidence upload.

## Impact

Limit: GitHub gives a cancelled job only a short grace period for post-cancel
steps, and a runner that is itself dead or unresponsive runs no step at all. This
helps healthy host cancels and time-limit cancels where the runner survives; it
does not recover diagnostics from a lost runner, and a partial upload may still be
cut off by the grace period. Cancelled partitions will now publish a
`*-coverage-diagnostics-*` artifact (when the diagnostics directory exists; the
step warns otherwise) under a name the merge's `partition-*` pattern does not
match, so merge inputs and the required-check result are unchanged. Costs a
few seconds on cancelled jobs only.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
