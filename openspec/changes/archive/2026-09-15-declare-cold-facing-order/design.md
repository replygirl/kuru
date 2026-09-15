## Context

The current final tie-break inherits `BTreeMap` UUID order. Built-in UUIDs are
stable but their sort order is unrelated to the authored framework order, so the
current `stable-identity` label hides an accidental cold-facing policy.

## Goals / Non-Goals

**Goals:** Use the maintainer-selected authored order only for unresolved cold
ties and emit the actual decision reason.

**Non-Goals:** Add authority, change targeting/focus/activation/continuity,
create a new focus interface, or extract the Phase 1 mode framework.

## Decisions

`Framework` exposes its existing ordered built-in identities. Speaker selection
intersects that order with the tied eligible draft identities after stronger
rules. A stable ID fallback remains for ties consisting only of dream-authored
parts or relationships.

## Risks / Trade-offs

- [Self could look supervisory] → apply authored order only to a cold tie and
  retain the equal-peer prompt and all stronger runtime rules.
- [Retired authored part] → intersect with eligible candidates and advance to
  the next authored identity without reviving anything.
