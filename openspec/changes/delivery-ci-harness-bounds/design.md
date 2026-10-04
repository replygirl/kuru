# Design

## Context

Root cause: the harness bounds individual commands, settles and launches with literals chosen by guess (30 s, 120 s, 180 s, 10 min) in addition to the shard deadline that already encodes the hosted job limit. When a literal is shorter than what the deadline allows, the harness fails a partition or records a failed run for slowness it was permitted to tolerate. `RETIRE_BOUND` cites a `SERVICE_IDLE_TIMEOUT` that does not exist (kuru-memory retires the owner at once on its last detach, bounded by the close budget).

## Decisions

- The shard deadline chain is the only source: a command, list or capture waits on its exit event bounded by `remaining_until(deadline)`; cleanup and settle bounds derive from the `EVIDENCE_RESERVE` slice kept between the shard deadline and the job limit, so they always fit inside it.
- Open-time bounds derive from the product's own budgets (startup and close budgets in kuru-memory/kuru-core) plus the platform process API's cleanup bound, each cited at the constant; where no product budget exists the wait takes the job deadline.
- A constant is deleted when the event it waits for already carries a bound; it is kept only with its derivation and a pin test of that derivation.
- The release workflow `tests` job records `KURU_COVERAGE_JOB_STARTED` and `KURU_COVERAGE_JOB_MINUTES` exactly as `ci.yml` and `native-tests.yml` do; the deadline arithmetic is unchanged.
- The full per-point table (what it bounds, governing budget, derived value or deletion, meaning of expiry) lives in the untracked `tmp/roadmap/store-creation-design/derivation-delivery-ci-harness.md`; the derivation at each constant is the durable record.

## Risks / Trade-offs

- Removing a short literal lets a genuinely hung child consume the partition's remaining time before failing, with the stall sampler's diagnostics rather than an early "did not settle". Accepted: it matches the shard deadline model, and diagnostics are retained on expiry.
- The inline-test race fixes replace wall-clock sleeps with observed events; they must not weaken what the tests assert.
- Open-time bounds move both ways. The run bound rises from 180 s to 345 s, because the old value was below the product's own 310 s first-project creation budget. The census query bound becomes that run bound, which lengthens the worst-case gate job; the 60-minute job limit still bounds it. The retirement bound falls from 120 s to 62 s, the owner's longest product retirement path, so an owner that outlives its own budgets stops the series sooner. Nothing is killed.
- The settle after an ordinary exit loses its 30 s bound. A settle still running at the shard deadline is reported as a stall. The settle step it was in is appended to the stall sample, because `StallReport`'s schema is pinned and gains no field.

## Operational surface

CI runner topology: the `tests` job in `.github/workflows/release.yml` (ubuntu, `timeout-minutes: 60`) gains a step writing `KURU_COVERAGE_JOB_STARTED` to `GITHUB_ENV` and sets `KURU_COVERAGE_JOB_MINUTES` on the test step. No new secrets, binaries or runner types.
