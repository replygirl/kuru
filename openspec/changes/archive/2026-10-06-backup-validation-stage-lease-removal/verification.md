# Verification

## 1. Settled image cleanup [critical]

- [x] 1.1 @regression (agent) unchanged historical_dirty_restore_migrates_actual_working_values_without_rewriting_image -> owning native task passed 1/1 locally with all actual WORKING migration, original-image and checked-cleanup assertions unchanged; original x64/ARM CI failed at occupied validation-stage removal, corrected native Windows cure remains required by fresh CI
- [x] 1.2 @unit (agent) existing backup_async_fixtures_use_the_closing_scope -> unchanged guard passed; owning historical+guard selection 2/2, zero ignored, 12.26s (task 82.72s), all other targets filtered

## 2. Scoped static acceptance

- [x] 2.1 @equivalence (agent) affected memory host/Windows lint, typecheck, docs, format and managed checks -> all six owning categories exited 0; first format check caught equivalent seal.remove_tree line wrapping, corrected after readers settled and final format passed; only reviewed ImageStage discard composition changes
- [~] 2.2 @e2e (agent) final-head native Windows and full PR/exact-main acceptance -> defer: required fresh public-head CI follows the archived local correction; no local Windows cure claim

## Original observed failures

Final `0efc1d9a` Windows x64 coverage6 job112597852521 and ARM behavior6
job112597925273 report historical dirty restore's uncertain validation-stage
removal: the removed directory name remains occupied. Windows x64 coverage1
job112597852501 reports the same chain from direct prepared-backup validation.
These failures are retained without unchanged retry. Managed backup/EOF/CLI
failures expose only generic errors and remain separately qualified.

## Ownership review

Root and Architecture verified the existing `LifecycleLease::remove_tree(self)`
consumes its checked directory while retaining the lock through removal; an
existing template-cleanup caller already uses that API. No early seal release,
platform absence relaxation or new process primitive is authorized.

## Gate and local acceptance

Actual strict/apply exited 0 and all four returned contexts were read before
implementation. Existing no-delta upstream advisory was retained; specifications
are unchanged, so archive explicitly skips spec synchronization. Root and
Architecture independently cleared the frozen source. No local native Windows
execution, paid run or broad local suite was performed.
