# Proposal

## Why

The new asynchronous HTTP catalog fixture starts outside the established whole-body memory closing scope. PR #250 Windows coverage partition 5 therefore fails the existing structural guard before accepting the suite, naming this exact test; the catalog body itself did not report an error.

## What Changes

Wrap the existing test body in `kuru_memory::test_support::closing(async { ... }).await`. Retain the same HTTP server abort and await, capability/call-count assertions, unchanged legacy sentinel and absent-memory assertions.

## Capabilities

### Modified Capabilities

None. The existing fixture teardown contract and explicit catalog behavior remain unchanged.

## Impact

Only `apps/kuru-tui/tests/trust.rs` and this fix record. No production, guard exemption, dependency, deadline, authentication or memory authority change. No design or spec delta is needed for the existing wrapper contract.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
