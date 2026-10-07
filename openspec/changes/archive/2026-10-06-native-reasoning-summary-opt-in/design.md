# Design

## Context

The shared `prepare_responses_request` adds an effort object but never the summary opt-in documented by the [official Responses reasoning guide](https://developers.openai.com/api/docs/guides/reasoning#reasoning-summaries). Encrypted continuation and reasoning-token usage do not request or establish a textual summary. Existing parsing already accepts streamed and terminal-only provider summaries.

## Decisions

Add `summary: "auto"` inside the existing reasoning object only when `request.effort` is present and differs from `none`. Keep the exact effort string, including future values. Absent effort keeps the reasoning object absent; explicit `none` remains effort-only. This avoids adding reasoning parameters to nonreasoning API models solely from their names. No catalog cache, new capability registry or runtime field is needed.

`Config::default().effort` is absent. CLI automatic model selection supplies the selected model's advertised default effort when available; a non-`none` default therefore opts in. Explicit model selection without effort and catalogs without an advertised default retain provider-default behavior without a summary request. Preflight's existing native fixture supplies advertised `low` explicitly and exercises the corrected request.

Estimate and send use this same body builder, retaining exact final-body accounting. Existing native continuation, summary coordinates, private storage, transient UI and public replay boundaries are unchanged.

## Integration contract

The API-key route and fixed ChatGPT subscription route remain separate authentication transports using shared Responses body preparation. `auto` is the documented provider-selected summary mode; Kuru does not synthesize summaries or expose raw reasoning. Captured local HTTP requests prove body shape and parser propagation. Official public Responses documentation does not establish acceptance by the undocumented subscription endpoint; the single existing bounded isolated live fixture will provide that functional evidence only after frozen source and review.

## Risks / Trade-offs

Provider emission is optional: successful reasoning or positive reasoning-token usage does not imply a summary. Retain honest unobserved evaluation evidence when none is returned. Unknown effort values remain provider-validated; no fallback between native and API-key access is introduced.
