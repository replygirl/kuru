# Verification

## 1. The marker wait no longer races the pipeline's unbounded work [critical]

- [ ] 1.1 @regression (agent) inject a brief artificial delay ahead of the first hook's dispatch (long enough to exceed the old flat 10s marker wait, well inside the new backstop) and run the test against the fixed code, then again against the unmodified old code -> fixed code passes, old code fails with `Elapsed(())` at the old marker-wait line; record both outcomes and timings, and note this exercises the failure mode on demand rather than reproducing the original loaded-runner race.
- [ ] 1.2 @unit (agent) run `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants` unmodified, several times in a loop, locally -> passes every run; record the observed wall-clock time for each re-derived wait as evidence the new bounds have real headroom over what was actually observed.

## 2. The dream-join wait no longer ties the product's own quiesce bound

- [ ] 2.1 @unit (agent) assert that the join wait's computed bound is strictly greater than the value returned by the new `HookHost` quiesce-bound accessor -> confirms the bound is derived, not an equal literal that would eventually tie the product's own worst case.

## 3. No behavioral regression elsewhere in the package

- [ ] 3.1 @integration (agent) run `mise run //packages/kuru-runtime:test` (full package suite) -> all tests pass; record the pass count and any failures.
- [ ] 3.2 @manual (agent) read the derivation comments on both re-derived bounds in the committed diff -> each states its terms and source (hook `timeout_ms`, the new quiesce accessor, and the named margins) rather than a bare literal.

## 4. Deferred — cannot be run from this change record alone

- [~] 4.1 @e2e (agent) observe the fixed test pass on the same macOS coverage partition under real CI load, the only environment the original failure was observed in -> defer: this change's apply step runs local cargo/mise commands only; CI execution happens on the next push and PR run, which this change record does not perform or rerun.
