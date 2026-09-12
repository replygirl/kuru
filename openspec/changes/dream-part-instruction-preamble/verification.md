## 1. Canonical instruction construction [critical]

- [x] 1.1 @regression (agent) run the focused core/runtime regression in which a valid dream `Add` formerly persisted its raw tendency -> native macOS baseline exited 101 at the intended missing-prefix assertion in `/private/tmp/kuru-dream-preamble-baseline-native.log`; corrected runtime filter exited 0, 1/1 passed in `/private/tmp/kuru-dream-preamble-runtime-regression.log`. Root reviewed the actual runtime Add path and exact canonical instruction assertion.
- [x] 1.2 @unit (agent) exercise unwrapped, singly wrapped, repeatedly exactly wrapped, prefix-like, and non-leading prefix-like UTF-8 tendencies through the core constructor -> focused core filter exited 0, 1/1 passed in `/private/tmp/kuru-dream-preamble-core-unit-final.log`; actual leading/trailing whitespace, a whitespace-prefixed full token, partial token, and repeated exact tokens preserve the declared tail bytes. Root reviewed unchanged builtin literals/identity seeds and the exact builtin instruction assertion.
- [x] 1.3 @unit (agent) exercise blank, prefix-only, partial-prefix, and near-8,192-byte tendencies -> the same final core filter passed blank and repeated-prefix-only rejection, nonblank partial-prefix preservation, exactly 8,192-byte raw UTF-8 acceptance, oversized wrapped resubmission rejection, and idempotent exactly 8,192-byte canonical input.

## 2. Dream persistence and reversibility [critical]

- [x] 2.1 @integration (agent) apply mixed valid and invalid `Add` proposals through the real candidate, promote it, stop and reload the store, then undo -> native macOS real-Dolt filter exited 0, 1/1 passed in `/private/tmp/kuru-dream-preamble-persistence-final.log`; accepted report retains its original repeated-prefix payload, invalid neighboring addition rejects individually, the stored new part is canonical through stopped reload and archived on undo. An existing unwrapped legacy part retains exact bytes through promotion, stopped reload and undo. Original role/rejection and later-history tests were retained and each passed 1/1 in `/private/tmp/kuru-dream-preamble-existing-rejections-final.log` and `/private/tmp/kuru-dream-preamble-existing-later-history.log`.
- [ ] 2.2 @integration (agent) run the native Windows real-memory variant of the focused dream persistence fixture -> required before merge: local execution is unavailable; supported Windows CI must observe the same persistence and reversal contract.

## 3. Structural behavior, not semantic policy

- [x] 3.1 @eval (agent) use fixture tendencies containing repeated exact tokens, similar prose, and non-leading prefix text -> the final core fixture above and root source review establish only exact leading-token normalization; no semantic classifier or denylist was added. This is deterministic structural evidence, not a model-behavior or prompt-injection-resistance claim.

Scoped format, core lint/typecheck and runtime lint/typecheck each exited 0 in
`/private/tmp/kuru-dream-preamble-format.log`,
`/private/tmp/kuru-dream-preamble-core-lint-final.log`,
`/private/tmp/kuru-dream-preamble-core-typecheck-final.log`,
`/private/tmp/kuru-dream-preamble-runtime-lint-final.log` and
`/private/tmp/kuru-dream-preamble-runtime-typecheck-final.log`.
Independent checkpoint coverage subsequently passed: `mise run coverage` in `/private/tmp/kuru-phase0-checkpoint` exited 0 (session 83966), 19,989/20,960 lines = 95.37%, `/private/tmp/kuru-phase0-checkpoint-coverage.log`.
The real canonical-add/promotion/stopped-reload/reversal fixture and original
runtime regressions ran successfully against the established memory backend.
Native Windows row 2.2 remains an uncompleted before-merge gate.
