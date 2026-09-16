## 1. Core catalog contract

- [x] 1.1 Add defaulted sourced metadata, route keys, exact-decimal price components, provenance labels, and a resolved-window API in `kuru-core`; verify focused core tests cover serialization and each precedence path.
- [x] 1.2 Add and validate the bounded embedded official catalog asset, including route/ID uniqueness, sources, limits, price syntax, tiers, and API-equivalent mappings; verify malformed fixture tests fail safely.

## 2. Configuration and public schema

- [x] 2.1 Add bounded fallback-window configuration and semantic validation without allowing it to override verified limits; verify parser and resolver tests cover built-in and configured assumptions.
- [x] 2.2 Publish `configuration.v1.schema.json` and document its parser-authoritative, strict-forward-compatibility contract; verify the static asset is linked from the configuration reference.
- [x] 2.3 Add Rust parser/schema parity tests using the current compatible test-only validator; verify defaults/examples pass and unknown keys or invalid bounds fail in both structural layers.

## 3. Connector enrichment

- [x] 3.1 Enrich Codex and official Responses live discovery through the core catalog while preserving every live model/effort string; verify advertised-over-snapshot precedence and unknown records.
- [x] 3.2 Keep custom Responses API bases route-isolated and Codex prices explicitly API-equivalent; verify route-isolation and exact-slug tests retain absent price when no verified basis exists.
- [x] 3.3 Update mechanical `ModelInfo` constructors in runtime, TUI, and tests; verify the affected workspace packages compile and focused tests pass.

## 4. Integrated verification

- [x] 4.1 Run the catalog, connector, and config/schema integration tests and record observed results in `verification.md`; verify every critical catalog and unknown-window scenario.
- [x] 4.2 Run docs build/content checks and record observed results in `verification.md`; verify the versioned asset reaches the built site with its documented reference.
