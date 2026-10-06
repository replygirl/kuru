# Proposal

## Why

Kuru's native ChatGPT subscription integration depends on an upstream wire contract that can change independently of Kuru's pinned source. The connector already validates complete streams and keeps subscription and API-key routes separate, but it does not distinguish a positively observed semantic contradiction from ordinary operational failures in a stable way. Operators also lack an explicit, bounded check for observing the subscription route when their account can participate.

## What Changes

- Classify only complete authentication, catalog, or inference responses that contradict a required semantic as connector incompatibilities with fixed, bounded reason codes. Incomplete streams, read/transport failures, expired credentials, rejected requests, absent optional usage/reasoning, and unknown compatible fields/model/effort values retain existing behavior.
- Preserve item-ID terminal reconciliation, accepted terminal shapes, same-item tool identity announcements, and complete JSON validation before dispatch. A positively incompatible completed tool response stops before ToolHost and never triggers redispatch or route fallback.
- Add an operator-invoked `kuru canary --model MODEL` for the fixed ChatGPT subscription route. It emits a versioned sanitized report of observed and unobserved stages and never activates workspace tools or memory.
- Keep live canary participation optional. An unavailable or unrun canary is unverified and is not a normal login, inference, CI, or release gate.
- Extend deterministic fake OAuth, HTTP, SSE, and fresh-process CLI coverage without reading personal credentials or making paid requests.

## Capabilities

### New Capabilities
- connector-compatibility: Classify proved native protocol contradictions and provide deterministic evidence plus an explicit optional subscription canary.

### Modified Capabilities
- provider-tools: Preserve separate native authentication/inference routes and existing retry/tool-admission behavior while surfacing only positively proved incompatibility.

## Impact

Changes are limited to `packages/kuru-connectors` authentication and provider decoders/diagnostics, `apps/kuru-tui` fixed CLI dispatch and fresh-process tests, and authentication/command documentation. The connector API gains finite typed incompatibility codes and a subscription-canary report operation. No dependency, credential migration, memory/runtime authority change, provider fallback, live account prerequisite, or route behavior change is introduced.

## Surfaces

- [x] interactive — an operator invokes and reads the canary report
- [ ] deploy — CI and release topology remain unchanged
- [x] integration — upstream OAuth/catalog/inference contracts are external
- [x] agent-behavior — incompatible completed tool responses stop before ambiguous dispatch
