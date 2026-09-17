## Why

The four validated built-in mode profiles still sit beside hard-coded runtime
dispatch, peer admission, relationship creation, and facing selection. Until
those calls flow through the profile operations, the mode contract cannot prove
that roles, peering, flow, and facing govern real execution while preserving
the equal-peer behavior users already receive.

P8 is the serial runtime integration stage after the permission, streaming,
typed-event, and accounting seams have settled. It gives policy decisions an
effect at their existing runtime boundaries without allowing a policy to own
provider calls, tools, memory, budgets, cancellation, or publication.

## What Changes

- Route fresh topology roles, deliberation scheduling, peer and relationship
  admission, consultation, shared contributions, and speaker selection through
  the existing validated Roles, Peering, Flow, and Facing operations.
- Add the narrow core relationship-proposal origin distinction (`User` or
  `Peer` with its checked context), so direct user/API relationship creation
  retains its current behavior without inventing a peer sender.
- Preserve byte- and behavior-equivalent outcomes for IFS, polyvagal,
  Freudian, and Jungian modes through golden runtime scenarios. Test-only
  alternate policies must demonstrably restrict or change the runtime seam
  before mail, events, provider work, or a selected-facing request occurs.
- Thread role-aware validation through the existing dream topology paths only.
  Dream consolidation, visibility, context selection, namespace routing, and
  memory policy remain P9 work.

## Capabilities

### New Capabilities

- `mode-policy-dispatch`: Runtime consumption of the Roles, Peering, Flow, and
  Facing decisions while retaining execution authority and four-mode parity.

### Modified Capabilities

- `mode-policy-contract`: Add explicit user-or-peer relationship proposal
  origin validation required by runtime policy dispatch.

## Impact

- `packages/kuru-core/src/policy.rs`: narrow typed relationship proposal origin
  and validation correction.
- `packages/kuru-runtime/src/engine.rs`, `dream.rs`, and runtime golden/seam
  fixtures: policy-driven routing and role-validation plumbing.
- Depends on archived `mode-policy-contract`, `typed-runtime-events`,
  `facing-provider-streaming`, `tool-permission-decisions`, and
  `context-usage-accounting` interfaces.
- No new mode, user control, configuration, UI, storage migration, provider,
  general policy executor, or change to P6 permission/P7 accounting authority.

## Surfaces

- [ ] interactive — no new user-facing control or layout
- [ ] deploy — no deployment or workflow topology change
- [ ] integration — no external service contract change
- [x] agent-behavior — runtime policy dispatch changes which existing peers are
  scheduled, admitted, consulted, and selected under the same built-in modes
