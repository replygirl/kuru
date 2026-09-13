# Dependencies

## Blocked by

- [x] `provider-response-budgets` — supplies the separate 600-second completion, 60-second catalog, and bounded SSE transport budgets consumed by this diagnostic reader *(archived 2026-09-12)*

## Soft-blocked by

None.

## Existing foundations

The archived OpenAI-authentication change provides the separate native ChatGPT
and API-key Responses transports. The archived workspace-trust and effective-Dolt
GC changes do not provide or consume provider diagnostic behavior. The public
documentation delta preserves the archived workspace-trust contract.
