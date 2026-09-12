## 1. Bounded Unix post-cleanup observation [critical]

- [x] 1.1 @regression (agent) exercise the private observer seam with a deterministic `EPERM` then `ESRCH` sequence -> native macOS baseline probe asserted the old immediate error and unconsumed ESRCH, 1/1 passed in `/private/tmp/kuru-process-group-baseline.log`; corrected observer filter passed 4/4 in `/private/tmp/kuru-process-group-observer-final.log`, requiring ESRCH before success. Root reviewed the query-only seam and unchanged kill sites.
- [x] 1.2 @unit (agent) exercise a persistent `EPERM` sequence through the fixed five-second observer deadline -> the same 4/4 filter passed in 5.03 seconds; persistent permission produced TimedOut with an EPERM diagnostic after the production deadline and never counted as absence.
- [x] 1.3 @unit (agent) exercise existing-process then `ESRCH`, and an unexpected observer error, through the same seam -> the same filter passed existing-then-absent success and immediate INVAL failure with the existing query diagnostic.

## 2. Owned native cleanup [critical]

- [x] 2.1 @regression (agent) `mise run //packages/kuru-delivery:test -- bounded_output_cleans_silent_descendant_before_reaping_successful_root` -> this real native macOS fixture passed within the broader bounded_output filter in `/private/tmp/kuru-process-group-bounded-output-final.log`, retaining its successful-output and descendant-cleanup assertions unchanged. This run does not claim that the operating system reproduced transient EPERM; row 1.1 establishes that transition deterministically.
- [x] 2.2 @integration (agent) `mise run //packages/kuru-delivery:test -- bounded_output` -> exited 0, 5/5 native macOS overflow, timeout, delayed-root, natural-exit and silent-descendant cases passed in `/private/tmp/kuru-process-group-bounded-output-final.log`.

## 3. Scoped static and platform boundary

- [x] 3.1 @integration (agent) `mise run //packages/kuru-delivery:lint` and `mise run //packages/kuru-delivery:typecheck` -> both exited 0 in `/private/tmp/kuru-process-group-lint-final.log` and `/private/tmp/kuru-process-group-typecheck-final.log`; format check also exited 0 in `/private/tmp/kuru-process-group-format-final.log`.
- [ ] 3.2 @integration (agent) native Windows execution -> unrun locally: this Unix-only observer change does not alter Windows process behavior; native Windows fixture execution remains required in CI

Independent checkpoint coverage also passed: `mise run coverage` in `/private/tmp/kuru-phase0-checkpoint` exited 0 (session 83966), 19,989/20,960 lines = 95.37%, `/private/tmp/kuru-phase0-checkpoint-coverage.log`.
The unchanged silent-descendant regression and bounded-output group cases ran
successfully; this does not claim the host reproduced transient EPERM.
