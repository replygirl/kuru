# Tasks

## 1. Typed positive incompatibility boundary

- [x] 1.1 Add finite connector incompatibility codes, a sanitized typed error, and an error-chain extractor; verify fixed code/action text and bounded length with sentinel values.
- [x] 1.2 Mark only missing required fields in successfully read and parsed bounded token responses; verify complete missing-field fixtures receive a code while expiry, rejection, malformed JSON, and transport/read failures do not.
- [x] 1.3 Mark only missing required catalog fields after successful bounded JSON parsing; verify unknown model IDs, effort values, and harmless metadata remain accepted.
- [x] 1.4 Map complete terminal identity/output contradictions to typed reasons without classifying malformed model-generated arguments or incomplete streams; verify accepted empty-terminal/item-ID/same-item announcement cases remain unchanged.

## 2. Fixed-route subscription canary

- [x] 2.1 Implement the bounded connector canary report using the existing ChatGPT credential manager and subscription provider; verify versioned finite state/reason/stage serialization and exit-code mapping.
- [x] 2.2 Add the explicit model-required CLI operation before workspace preflight and memory/tool activation; verify command misuse retains exit 1 and no Responses provider is constructed.
- [x] 2.3 Add fresh-process loopback-fake CLI cases for compatible, incompatible, and inconclusive runs; verify exact request counts, sanitized JSON, no tool effect, no API-key fallback, and no workspace authority/memory activation.
- [x] 2.4 Document the explicit canary and its optional, stage-limited evidence; verify authentication and command docs state that no live canary is required for ordinary use, CI, or release.

## 3. Contract evidence and scoped integration

- [x] 3.1 Extend isolated native fake-service coverage for login/refresh, catalog, terminal stream, tool identity, reasoning, usage, and route separation; verify accepted future fields remain supported and positively incompatible completion stops before dispatch without redispatch.
- [x] 3.2 Run the owning connector/TUI format, lint, typecheck, focused tests, docs, and strict/managed Cospec checks; verify each actual local command passes or its exact outstanding failure is recorded.
