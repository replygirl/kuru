## Why

The four validated mode profiles still leave Visibility and Memory as passive
contracts while the runtime reads fixed context, namespace, delivery and dream
paths. Until those two axes are consumed at their existing effect boundaries,
the Phase 1 mode foundation cannot prove private-context isolation, request-fit
accounting, or candidate-safe memory behavior under a policy decision.

This is the final Phase 1 mode-dispatch slice after P8. It makes the remaining
two existing axes effective without introducing a mode format, policy parser,
or a second runtime authority.

## What Changes

- Route every actual actor request through validated Visibility context sources
  and delivery decisions, preserving mandatory current input, receipts and
  continuation material and P7's whole-request fit accounting.
- Route actor, note, transcript, topology and dream state paths through the
  validated Memory namespace and state-key decisions; use the memory
  consolidation plan for each actual dream while the runtime retains candidate,
  reconciliation, cancellation and publication ownership.
- Add only the narrow core validation needed to require `ExplicitInput` and to
  validate an ordered, unique live consolidation-participant subset with the
  existing two-proposal ceiling at each use.
- Preserve byte- and behavior-equivalent IFS, polyvagal, Freudian and Jungian
  outcomes, P6 permission enforcement, P7 accounting, P8 dispatch, retries,
  persistence-before-publication, cancellation and equal peers.

## Capabilities

### New Capabilities

- `mode-visibility-memory`: Runtime dispatch of the existing Visibility and
  Memory policy axes with validated private context, namespace and dream-plan
  effect boundaries.

### Modified Capabilities

- `mode-policy-contract`: Require mandatory explicit input and validate the
  bounded consolidation-plan result that P9 consumes.

## Impact

- `packages/kuru-core/src/policy.rs` and policy tests: narrow source and
  consolidation-plan validation.
- `packages/kuru-runtime/src/actor.rs`, `engine.rs` and `dream.rs`: policy
  consumption and runtime validation at existing context, delivery, namespace,
  candidate and undo boundaries.
- Runtime and real-Dolt fixtures plus final Phase 1 verification and owning
  documentation: built-in parity and local evidence. No migration, new mode,
  user control, provider, permission rule, storage format or UI is introduced.

## Surfaces

- [ ] interactive — no new user-facing control or layout
- [ ] deploy — no deployment or workflow topology change
- [ ] integration — no external service contract change
- [x] agent-behavior — policy-selected context and memory routing affect
  existing provider requests and peer delivery
