# Dependencies

## Blocked by

- [x] `native-platform` — safe native browser/process boundaries and the package that owns OS credential-store mechanics *(archived 2026-09-10)*
- [x] `workspace-trust` — exact-root preflight and immutable authority-manifest binding before configured MCP activation *(archived 2026-09-12)*
- [x] `phase2-config-instructions` — typed configuration/provenance layers and strict published-schema parity *(archived 2026-09-22)*
- [x] `command-registry` — shared working CLI/TUI help, completion, parsing and dispatch registry *(archived 2026-09-22)*
- [x] `mcp-catalog-controls` — alias-local activation, static-header separation, cached metadata and shared status projection *(archived 2026-09-22)*

## Soft-blocked by

None.

## Siblings

`openai-authentication` and `oauth-transport-contract` provide the existing ChatGPT-specific authentication route and documented support contract. P13 must preserve their browser handoff behavior and credential isolation, but MCP OAuth uses its own standards, commands and native-store records and does not depend on or import that route.

`mcp-catalog-integrated-acceptance` exercises the P12 mixed-state CLI/TUI catalog on this branch. It is preserved as integration evidence rather than a product prerequisite.
