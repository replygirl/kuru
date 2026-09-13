## Why

Equal peer activations currently resolve through an offset derived from the turn
counter, causing the speaking identity to rotate even when nothing relevant has
changed. Phase 0 calls for speaker stickiness and an explicit tie-break, and the
roadmap author asks that the correction remain observable without introducing a
new mode-policy system.

## What Changes

- Preserve existing caller target and active focus behavior.
- Among eligible deliberating peers, select the greatest activation; retain the
  previous successfully completed speaker when it shares that maximum, otherwise
  use a stable identity ordering independent of the turn counter.
- Persist the last completed speaker as optional session state, accepting old
  session records without that field. Failure before completion publication
  leaves continuity unchanged; uncertain publication uses existing reconciliation.
- Include a bounded selection reason in the existing event trace and document
  the rule. Keep the existing speaker event and TurnOutput fields compatible.

## Capabilities

### Modified Capabilities

- `peer-cognition`: deterministic continuity when peer activations tie.

## Impact

The runtime owns selection and persisted session state. Runtime tests and user
documentation change alongside it; no dependency, SQL schema, provider, new
command, framework-policy interface, or new user setting is required.

## Surfaces

- [x] interactive — the speaking identity no longer changes solely with turn count
- [ ] deploy
- [ ] integration
- [x] agent-behavior — deterministic selection and its event reason
