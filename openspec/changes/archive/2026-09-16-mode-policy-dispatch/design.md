## Context

P3 provides pure, validated mode profiles and a four-mode pre-extraction
baseline, while `engine.rs` still owns the hard-coded dispatch loops. P8 is the
single writer for the engine extraction stage; P9 follows it for visibility,
namespace, and dream consolidation routing. P6 and P7 have already established
the permission and accounting boundaries the runtime must preserve.

## Goals / Non-Goals

**Goals:**

- Consume the first four policy axes at the engine decisions they already
  describe, preserving four-mode byte and behavioral parity.
- Keep every policy output untrusted until the runtime validates it against
  current topology, pending work, and existing execution limits.
- Correct the relationship proposal input so User/API and Peer proposals have
  honest, non-interchangeable origins.

**Non-Goals:**

- Visibility, memory, context-source, namespace, and dream-consolidation
  routing; P9 owns those decisions.
- Shipping policy composition, new modes, configuration, UI controls, a
  supervisor, or an execution callback exposed to policies.

## Decisions

### Consume decisions at existing effect boundaries

`engine.rs` will call Roles when reading and validating topology, Flow while
constructing initial/next recipients and contribution envelopes, Peering before
peer/relationship effects, and Facing immediately before speaker dispatch.
The alternative of letting policies mutate a harness or return a loop was
rejected because it would blur provider, memory, permission, and publication
authority that must remain runtime-owned.

### Treat every policy result as an input to runtime validation

Recipient decisions are checked against live actors and actual pending IDs;
direct and relationship decisions are checked before an event or mailbox write;
and facing decisions are checked against eligible target/focus/draft rules.
When an explicit target is supplied, the initial recipient vector must equal
`[target]`, and Facing must retain that target. Any other result is a refusal,
never a silent reroute to a different live peer.
The alternative of trusting built-in equivalence was rejected because internal
test profiles must prove restrictive behavior without gaining a bypass.

### Publish a selected profile only with its durable state

The selected built-in or test profile is held with harness state and changes
with the existing candidate configuration/topology/session publication path.
The alternative of replacing the profile before a checkpoint/reconcile outcome
was rejected because an uncertain write could expose a policy that persistence
did not accept.

### Keep dream integration to role validation

P8 threads role-aware topology checks through dream and undo paths. Moving
candidate consolidation or namespace decisions here was rejected because P9
must separately prove visibility and memory policy effects without a second
engine rewrite.

## Risks / Trade-offs

- [Built-in parity can drift during extraction] → retain normalized four-mode
  golden fixtures that compare requests, deliveries, reasons, limits, events,
  and durable outcomes before and after the routing change.
- [A policy result can name invalid live state] → validate at each effect
  boundary before dispatch, publication, or persistence.
- [Dispatch changes could bypass P7 accounting] → retain its admission and
  settlement paths and exercise policy-routed calls with accounting fixtures.
