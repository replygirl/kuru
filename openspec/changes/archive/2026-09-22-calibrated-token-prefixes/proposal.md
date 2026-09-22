## Why

Kuru currently estimates input tokens as half the serialized request bytes. That is explicitly approximate and can misjudge multilingual text, code, tool schemas and opaque native continuation even when the connector measures the final request. The runtime also places actor identity and changing conversation material before common instructions, limiting reusable prompt prefixes across otherwise similar requests.

P07 prepares reliable context selection before compaction work. It must improve request-fit decisions without inventing exact tokenizer mappings, changing provider access, or claiming cache savings without reported usage.

## What Changes

- Introduce a route- and model-aware sizing decision at final connector serialization. Use a pinned local tokenizer only for exact model IDs whose mapping is verified; keep an explicitly labelled conservative estimate for unknown models, custom Responses routes and opaque content. Calibrate against reported usage and, when already authorized and available, the official direct-API token-count endpoint in offline evaluation only. No runtime count request is introduced.
- Reorder prompt construction so genuinely shared, stable peer/project instructions and deterministic common tool definitions lead the request. Actor identity, phase, topology, public transcript, private histories, notes, state and native continuation remain after the shared prefix, within their existing visibility boundaries.
- Report actual cached-input usage from provider responses and compare request shapes; retain missing-versus-zero usage and incomplete cache-write price labels. Subscription caching support is not inferred from the explicit API-key Responses route.

## Capabilities

### New Capabilities

- `prompt-prefix-reuse`: Stable shared request prefixes and evidence required to report cache reuse.

### Modified Capabilities

- `context-usage-accounting`: Route-aware effective-request estimates, calibration and honest unknown fallback.
- `model-catalog`: Exact, sourced tokenizer mapping eligibility separate from model availability, context and price facts.

## Impact

Changes affect `packages/kuru-core` context/model metadata, `packages/kuru-connectors` final wire measurement, `packages/kuru-runtime` instruction assembly, and user documentation. A tokenizer dependency and assets, if justified by verified mapping, must receive exact workspace pins and license review before implementation; no new runtime network dependency or credential route is planned. Public completion and usage formats remain compatible.

## Surfaces

- [x] interactive — visible context estimate and cache evidence labels
- [ ] deploy — no service or release topology change
- [x] integration — provider wire sizing and tokenizer asset provenance
- [x] agent-behavior — prompt ordering while preserving visibility and authority
