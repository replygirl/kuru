# Proposal

## Why

PR #233's corrected head overflows the Windows test thread stack while the existing real slash-command fixture dispatches runtime commands. Its official coverage-partition log reports `STATUS_STACK_OVERFLOW`, while the separate runtime targets pass; the command operation embeds the enlarged dream/reconciliation futures in every caller's async state.

## What Changes

Heap-pin the existing private controlled command operation at its entry boundary. Preserve its signature, accepted operation, cancellation, command results and cleanup; do not increase stack sizes or deadlines or add an executor.

## Capabilities

### Modified Capabilities

None. Existing command and supported-platform contracts remain correct.

## Impact

`apps/kuru-tui/src/ui.rs` private dispatch only, plus this fix's required artifacts. The existing real slash-command fixture and mandatory closing guard provide local regression acceptance. The archived N3 and U2 records remain immutable; no protocol, schema, dependency, configuration or public API changes.

## Surfaces

- [x] interactive — preserve real slash-command behavior under a bounded caller stack
- [ ] deploy — no CI or runtime topology changes
- [ ] integration — no external contract changes
- [ ] agent-behavior — no prompt, routing or output changes
