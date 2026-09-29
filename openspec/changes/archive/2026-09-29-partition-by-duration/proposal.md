# Proposal

## Why

A run lasts as long as its slowest partition, and the name hash that assigns
tests to partitions ignores duration. On main run 36592401648 the Windows x64
partitions ran 602 to 959 s and Windows on Arm 695 to 862 s, because the
slowest single tests (139 s, 122 s, 100 s, 98 s on Windows x64) and the
slowest kuru-memory chunk shared partitions by chance.

## What Changes

This is repository and CI tooling under `kuru-delivery`'s unshipped `tooling`
feature, as in #118 and #129. No application source, spec or workflow file
changes.

- `packages/kuru-delivery/src/coverage/timing.rs` (new): a strict parser for a
  checked-in timing table, a canonical digest, a greedy longest-processing-time
  placement per OS label, gap attribution of recorded completions, and the
  table regeneration and staleness reports.
- `packages/kuru-delivery/src/coverage/timings.tsv` (new): one row per artifact
  key and test name, with attributed milliseconds per hosted OS label. The
  first table is the mean of main runs 36614320897 and 36592401648.
- `packages/kuru-delivery/src/coverage/partition.rs`: scheme `timed-lpt-v1`.
  A test with a table row goes to its placed partition. A test without a row
  keeps the unchanged SHA-256 name hash. The scheme binds the table digest, so
  partitions and the merge must use the same table.
- `packages/kuru-delivery/src/coverage/plan.rs`, `coverage.rs`: the runner
  records each exact selection's per-test completion offsets in the runner
  ledger. The record is informational. It is checked only for names inside
  the selection, no repeats, and completions no later than one second past
  the invocation's recorded run time.
- `packages/kuru-delivery/src/coverage/merge.rs`, `orchestrate.rs`: the merge
  reports predicted and measured seconds per partition, listed tests without a
  row, and rows without a listed test. It writes them to the job summary and
  warns (never fails) above a stale threshold.
- `packages/kuru-delivery/src/main.rs`, `mise.toml`: `coverage timings`
  (`coverage:timings`) regenerates the table from downloaded partition
  evidence, refusing evidence that is not one complete run per label. `coverage balance` prints the predicted partition totals of the
  checked-in table for each OS label.
- `docs/development.md`: partition assignment and table refresh.

## Impact

- Which tests run in which partition job changes on every OS. Which tests run,
  partition counts, job limits, receipts' agreement fields, the disjoint and
  complete proof and the 90% gate do not change.
- No secret, required check, workflow file or network access changes.
- Evidence from an older helper is refused, since its scheme differs. The
  partitions and the merge of one run share a commit, so they agree.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
