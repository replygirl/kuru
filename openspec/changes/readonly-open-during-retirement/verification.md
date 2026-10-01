# Verification

## 1. A read-only inspection after owner retirement reads its own generation [critical]

- [ ] 1.1 @regression (agent) `managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation` against a real owner paused after endpoint retire and after reap -> not yet run; must pass with a managed read-only open and fail for a borrowing open
- [ ] 1.2 @integration (agent) `mise run //apps/kuru-tui:test` preferences tests with the converted fixture -> not yet run; both pass
- [ ] 1.3 @runtime (agent) repeated local runs of `an_explicit_framework_override_is_validated_against_its_own_part_budget` -> not yet run; all pass (the unforced local baseline was 20 of 20 before the fix, so this alone does not prove the fix)

## 2. Contract and same-class sites

- [ ] 2.1 @unit (agent) lint and docs checks over the corrected facade comment and converted fixtures -> not yet run
- [ ] 2.2 @manual (human) review that `terminal.rs` live-owner borrows are unchanged -> not yet run

## 3. Platforms

- [~] 3.1 @runtime (agent) native Windows and ubuntu coverage runs -> defer: no Windows evidence was collected locally; CI is the only authority for ubuntu coverage timing
