# Design

## Context

The provider already owns separate ChatGPT subscription and Responses API-key routes, bounded HTTP/SSE reads, terminal item-ID reconciliation, and the final tool-admission boundary. Its decoders currently return generic errors for both semantic contradictions in successful completed responses and operational failures. The CLI has fixed early branches for ChatGPT login/logout before workspace configuration is activated, which is the correct boundary for an optional account canary.

The accepted stream rules remain authoritative: a complete terminal output controls item ordering and presence, previously visible text must agree, and one tool item's identity may be announced on any event for that same item. Unknown harmless provider metadata/model/effort values and absent optional usage/reasoning remain supported.

## Goals / Non-Goals

**Goals:** Add a small typed boundary for positively established contract contradictions; keep all operational error and retry behavior stable; ensure the optional canary uses the same connector catalog/decoder as ordinary inference; make each canary result stage-specific and sanitized.

**Non-Goals:** A new provider, route negotiation/fallback, generic diagnostics registry, compatibility daemon, automatic startup/release probe, or live paid-account requirement.

## Decisions

### Use finite typed incompatibility reasons at semantic boundaries

Add one small connector-owned compatibility type with a finite code set and an extractor for existing `anyhow::Error` chains. Mark a missing required field only after a successful bounded JSON response has been completely read and parsed; mark inference incompatibility only when a complete terminal response proves a required identity/output contradiction. Keep body-read/parse failures, incomplete streams, token expiry, rejected HTTP status, rate limits, and malformed model-generated tool arguments on their existing operational or validation paths. This is preferable to classifying generic decoder errors, which would mislabel transport and model-output failures as upstream contract drift.

### Preserve existing normalization and dispatch boundaries

Extend the current auth/catalog/SSE decoders rather than adding a second parser. Accepted terminal item ordering, empty terminal-envelope behavior, item-ID text agreement, same-item tool announcements, complete argument parsing, unknown model/effort metadata, and optional usage/reasoning remain unchanged. An incompatibility ends the provider operation before a `Completion` can reach ToolHost; no retry or route substitution is added. This keeps the compatibility judgment beside the evidence and avoids duplicating semantics in the canary.

### Make the canary a fixed early CLI operation

Add `canary --model MODEL` alongside the CLI's existing fixed ChatGPT account operations, before workspace configuration, tool authority, or memory activation. Reuse `AuthManager` and the existing subscription provider/catalog/decoder, with one bounded no-tool request, existing response caps, and an explicit end-to-end deadline. Never construct the Responses API-key provider or ToolHost. Return versioned JSON containing only finite state/reason/stage values; map verified, incompatible, and unverified to exit codes 0, 3, and 2 while preserving ordinary argument misuse exit 1. A no-run report is unverified. The alternative—automatic checks or a live gate—would make credentials and paid inference prerequisites for unrelated usage.

### Keep evidence deterministic and account-independent

Use connector-local fake OAuth/HTTP/SSE endpoints and fresh-process CLI tests with synthetic private credentials. Fake test endpoint injection is loopback-only and test-support-only. Assert exact route counts, no dispatch for contradictory calls, no fallback to API-key credentials, and no raw body/secret text in typed diagnostics or report JSON. Live account participation is documented separately and can remain unverified.

## Operational surface

The canary runs only in the user's ordinary Kuru process and makes bounded
outbound HTTPS requests to the existing ChatGPT subscription endpoints. It
opens no listener, container, or external service, and adds no binary or
credential dependency. It reads only Kuru's own checked ChatGPT credential
store, requires an explicitly selected model, and runs only after operator
invocation. Normal commands, CI, and release jobs never invoke it.

## Integration contract

The TUI CLI owns the explicit command, early fixed-operation dispatch, sanitized
report output, and exit code. `kuru-connectors` owns its finite incompatibility
codes, native `AuthManager`, subscription endpoint, existing model catalog and
SSE decoder, and canary observations. The Responses API-key route remains
separate. Production uses the existing pinned endpoints; fake endpoint injection
is restricted to test-support loopback fixtures. Upstream IDs remain opaque and
item-ID reconciliation continues to use the terminal response as authority.

## Risks / Trade-offs

- [Operational failures could be mislabeled as drift] → Construct typed reasons only after successful bounded complete response evidence; retain generic operational errors for incomplete, rejected, or unreadable responses.
- [Compatibility diagnostics could leak upstream content] → Emit fixed code/action text only; test with body and credential sentinels.
- [Canary could overstate the protocol it observes] → Report observed/unobserved stages independently; a successful no-tool exchange does not claim refresh, tool, reasoning, or absent-usage behavior.
- [Canary could accidentally activate project authority or billing fallback] → Dispatch it before workspace preflight and construct only the fixed ChatGPT subscription provider; verify with a fresh process and route-counting fake service.
