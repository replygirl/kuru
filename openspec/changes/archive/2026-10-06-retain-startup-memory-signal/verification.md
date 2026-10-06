# Verification

## 1. Listener continuity [critical]

- [x] 1.1 @regression (agent) queue the previously registered invocation signal before memory admission -> task96381 reproduced the former fresh-listener behavior and exited101: operation returned Ok(7), so required refusal failed. Corrected post-gate task5967 exited0; queued signal refused before operation poll. This causal production-helper case does not claim a new OS-signal/import-Dolt run.
- [x] 1.2 @unit (agent) signal only after the owned operation is polled -> task5967 exited0, two focused cases passed; callback released the held same operation and confirmed receipt was returned only after settlement.
- [x] 1.3 @integration (agent) run the established closing-scope structural guard -> initial task86601 exited0, helper2/2 and guard1/1 before separate fix gate; later only corrected helper2 were rerun after gate. Guard inputs/closing wrappers remained unchanged.

## 2. Integrated source and records

- [x] 2.1 @equivalence (agent) inspect the normal merge and prior archives -> current main794 already an ancestor of19287;03a56 is the sole missing M2 commit beyond common7a84. Normal merge exposed only commands.md conflict; resolution retains current update guidance plus inventory/import. CLI/store merged additively. Prior archive diff against19287 is empty except exact M2 archive additions, which equal03a56 bytes. No schema/wire/pin changes or M1/U4 WIP imported.
- [x] 2.2 @integration (agent) run affected package host/Windows statics, docs, format, strict and managed checks -> all terminal0: app host92282/Windows63401, memory host17166/Windows78521 (all targets/features, compilation included), full docs17855, root format22088, all52 strict and managed97323. No dependency pins/locks, schema or wire changes. Native cross-target lint is not native behavior acceptance.
- [~] 2.3 @e2e (agent) native full matrices, 90% coverage, release and paid models -> defer: later integrated delivery; these private helper cases establish receiver continuity and settlement, not a new OS-signal/import-Dolt pass.

Source and task86601 were drafted/launched before this separate fix gate; no preimplementation gate is claimed. Initial strict failed required Operational surface and unknown owner(CI); both corrected. Actual strict/apply then exited0 clear, five returned contexts read before baseline/corrected regression runs. Apply advisory no-delta-spec warning is handled through documented archive --skip-specs because existing specification is correct. Actual archive/normal commit/hooks follow this completed local acceptance; they are deliberately not preclaimed in this immutable record.
