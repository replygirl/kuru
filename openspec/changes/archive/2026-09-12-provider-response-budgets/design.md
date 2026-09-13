## Context

The API-key route uses the shared 60-second client, unlike the subscription
route's 600-second completion operation. The SSE decoder treats the shared 2
MiB retained-response limit as a cumulative transport counter, so ignored
events consume the completed response's budget and its line/data buffers have
no independent parser bound.

## Goals / Non-Goals

**Goals:**

- Give both Responses completion routes one 600-second total operation budget.
- Keep a 2 MiB retained final response while independently bounding stream
  wire, parser-line, and event-payload resource use.
- Preserve current route, authentication, error-redaction, and partial-stream
  no-replay behavior.

**Non-Goals:**

- Retries, refresh changes, new configuration, public streaming messages, or
  dependency updates.

## Decisions

- Set the 600-second deadline per API-key completion request, preserving the
  shared client's 10-second connect bound and 60-second catalog, MCP, and auth
  deadlines.
- Limit total SSE wire traffic to 64 MiB, a generous finite allowance for
  token/framing expansion; limit an event and its maximum line to 4 MiB so a
  one-line retained item and its completed-envelope duplicate can be decoded.
- Account retained completed items and the reconstructed final response against
  the existing 2 MiB cap. Ignore deltas and comments for retained accounting,
  but charge them to the wire limit. Validate the reconciled response after
  replacing an equivalent completed envelope output.

## Risks / Trade-offs

- [Very unusually verbose valid streams can exceed the 64 MiB wire guard] →
  The guard is explicit, finite, and far above the retained cap; streaming
  progress is not exposed by this private API.
- [Per-request timeout could accidentally broaden non-completion work] →
  Apply it only to the Responses POST and retain explicit 60-second catalog
  deadlines.

## Integration contract

The external Responses API continues to receive the same request and returns
the same completed JSON shape. Local fixtures use loopback servers and fake
secrets; they do not contact inference services or read credential stores.
