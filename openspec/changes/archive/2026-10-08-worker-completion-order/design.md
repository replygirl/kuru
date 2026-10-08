# Design

## Context

Native send wakes its receiver synchronously. Sending a confirmed result before
dropping its lease or hold lets that caller return against stale ownership state.

## Goals / Non-goals

Fix completion ordering within the existing native owners and preserve checked
cleanup across cancellation. No new runtime, registry, timeout, provider behavior,
tool permission, prompt or wire-format policy is introduced.

## Decisions

Confirmed hook workers drop their budget lease and tracking slot before sending.
Unix shell completion owns and releases its invocation hold before removing its
registry entry. Confirmed Windows shell paths do the same before replying.
MCP replies may carry the completed operation hold to their receiver; the public
operation drops it before returning. Failed delivery keeps that same hold until
the native worker cleans up. Unconfirmed paths retain their original owners and
holds after the bounded refusal, until checked cleanup finishes.
The memory selection worker releases only its temporary native lease clone before
replying. Pending/current state retains the exact lease during publication and
uncertain-outcome reconciliation; no claim or authority is inferred from counts.

## Integration contract

Native hook JSON, shell capture and MCP JSON-RPC shapes are unchanged. Connector
workers retain platform-owned children and reviewed directory handles; test
fixtures launch real owned children and observe lease, registry and hold state
at result publication or public completion. Private channel envelopes are not
protocol messages. Existing cleanup uncertainty and caller-loss paths keep the
original native owner and immutable per-operation hold. Memory workers already
joined before return and explicit lock/server handoffs remain unchanged.

## Risks / Trade-offs

Premature release during unconfirmed cleanup would break session draining; native
retention fixtures must continue proving that exact ownership survives. Tests
observe synchronous channel wake or completed API futures, so no timing allowance
can make an incorrect publication order pass.
