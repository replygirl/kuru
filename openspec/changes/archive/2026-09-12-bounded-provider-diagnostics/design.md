## Context

The generic connector HTTP reader rejects non-success status before reading a
provider diagnostic body. A separate Responses completion path formats
error.message and unexpected status strings from HTTP-success JSON envelopes, and raw reqwest errors can
carry endpoint URL/query context through anyhow chains. Native ChatGPT uses a
private SSE contract; current official Codex source observes a small set of
failed-event codes but does not establish a public schema promise.

## Goals / Non-Goals

**Goals:**

- Provide finite, actionable fixed diagnostics for supported provider failures.
- Bound provider diagnostic reads by 8 KiB and two seconds within the current
  60-second catalog and 600-second completion budgets.
- Prevent remote text and request URL/query values from entering any rendered
  or chained error.

**Non-Goals:**

- Retry, backoff, refresh/logout, credential handling, route selection,
  provider configuration, public completion types, or generic MCP/A2A error
  behavior.

## Decisions

- A provider-private classifier accepts only operation, HTTP status where
  present, source kind, and closed allow-listed code/type values. It constructs
  a new fixed error without retaining a reqwest or parsing source.
- HTTP-success envelopes with an unexpected status use fixed protocol text;
  their arbitrary status value is never interpolated. Full Display and Debug
  formatting of every exposed anyhow chain must preserve this boundary.
- Responses body mappings require their stated status; model_not_found only
  maps for 400/403/404. HTTP-200 error envelopes cannot claim 429 quota/rate
  meaning and use a fixed protocol failure unless a source-appropriate mapping
  exists.
- Native ChatGPT failed events map only the observed closed code set to fixed
  strings. Unknown code/type/message values become a fixed failed-stream or
  protocol failure. This uses current Codex source as evidence, not as a
  stability contract or copied implementation.
- The two-second reader deadline is a nested upper bound, not a new allowance:
  its elapsed time remains inside the already-running catalog or completion
  total deadline. Any read failure falls back to the already-known status class.
- Request send failures retain a private typed classification for future policy,
  but their rendered errors have no reqwest source. The classification MUST NOT
  infer an unsent request from connect-related or any other transport outcome,
  and it MUST NOT authorize replay.
- Client initialization and API environment lookup failures also discard their
  value-bearing source errors. Missing, empty and non-Unicode environment errors
  use fixed text without interpolating the configured variable name; lookup and
  authentication behavior remain unchanged.

## Finite mappings and evidence

The following is the complete initial mapping. It was reviewed on 2026-09-12
against the primary OpenAI sources linked below. Any value outside these literal
sets receives only its fixed status, protocol, or stream fallback; no arbitrary
code/type/message is rendered.

| Source | Recognition | Fixed diagnostic |
| --- | --- | --- |
| Responses HTTP | 400 or 422 | operation request was rejected (HTTP NNN) |
| Responses HTTP | 401 | operation authentication was rejected (HTTP 401) |
| Responses HTTP | 403 | operation access was denied (HTTP 403) |
| Responses HTTP | 404 | operation resource was not found (HTTP 404) |
| Responses HTTP | 400, 403, or 404 plus error.code model_not_found | operation selected model is unavailable or access is denied (HTTP NNN) |
| Responses HTTP | 429 plus error.code rate_limit_exceeded | operation is rate limited (HTTP 429) |
| Responses HTTP | 429 plus error.type insufficient_quota or error.code credit_balance_exhausted, organization_usage_limit_exceeded, organization_spend_limit_exceeded, or project_spend_limit_exceeded | operation is blocked by an API quota or billing limit (HTTP 429) |
| Responses HTTP | 429 without listed quota signal | operation is rate limited (HTTP 429) |
| Responses HTTP | 500–599 | operation service failed (HTTP NNN) |
| Responses HTTP | any other non-success status | operation failed (HTTP NNN) |
| Responses HTTP-200 envelope | non-null error object | operation returned a provider error response |
| ChatGPT SSE | context_length_exceeded | ChatGPT completion exceeded the model context limit |
| ChatGPT SSE | insufficient_quota | ChatGPT completion is blocked by a subscription quota limit |
| ChatGPT SSE | usage_not_included | ChatGPT subscription does not include this usage |
| ChatGPT SSE | cyber_policy, misalignment_policy_violation, or bio_policy | ChatGPT completion was blocked by policy |
| ChatGPT SSE | invalid_prompt | ChatGPT completion request was rejected |
| ChatGPT SSE | server_is_overloaded | ChatGPT service is overloaded |
| ChatGPT SSE | rate_limit_exceeded | ChatGPT completion is rate limited |
| ChatGPT SSE | unknown terminal failure | ChatGPT completion stream failed before completion |

Primary evidence reviewed on 2026-09-12: the [OpenAI error-code guide](https://developers.openai.com/api/docs/guides/error-codes), [OpenAI rate-limit guidance](https://help.openai.com/en/articles/5955604), and current [official Codex Responses SSE decoder](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/sse/responses.rs). The Codex decoder is an observed implementation reference for the closed native SSE set, not a stable public contract.

## Risks / Trade-offs

- A provider can return a useful but unknown code or message. -> Fixed status
  diagnostics remain safe and actionable; recognition expands only after a
  separately reviewed trusted source supports a literal.
- A slow failed body loses code-specific classification after two seconds. ->
  Return the immediate status class without extending the operation budget.
- Removing reqwest sources reduces support detail. -> It prevents accidental
  disclosure; a future support-log feature needs its own retention/redaction
  design.

## Integration contract

Responses JSON failures and the native ChatGPT SSE endpoint remain provider
contracts. Local HTTP and raw-SSE fixtures use only fake secrets; no live
credentials or provider calls establish verification evidence.

## Operational surface

No bind address, container setting, secret, binary, architecture, or deployment
limit changes. The terminal and CLI receive only fixed provider diagnostic text.
The provider reader's 8 KiB and two-second limits remain inside the established
per-operation 60-second catalog and 600-second completion limits.
