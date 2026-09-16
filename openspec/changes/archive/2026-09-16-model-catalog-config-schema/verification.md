## 1. Route-aware catalog enrichment [critical]

- [x] 1.1 @integration (agent) run core and connector catalog tests with fake live records and the embedded snapshot -> observed: `cargo test -p kuru-core --test model_catalog --test config_schema` passed 9 tests and connector focused tests preserved unknown live IDs/efforts
- [x] 1.2 @integration (agent) run catalog validation cases -> observed: malformed-catalog tests passed for nonpositive, duplicate, unsourced, invalid-decimal, and invalid route/price-basis fixtures; a valid sourced zero rate remained present
- [x] 1.3 @integration (agent) run route-isolation and price-basis tests -> observed: connector official/custom route test and core Codex test passed; custom base has no official facts and exact Codex slug has labelled API-equivalent tier terms

## 2. Unknown-model context behavior [critical]

- [x] 2.1 @integration (agent) run resolved-window tests through public core APIs -> observed: resolved-window and mixed-source tests passed for built-in/configured assumptions, verified precedence, and omitted conflicting snapshot facts

## 3. Configuration schema parity [critical]

- [x] 3.1 @integration (agent) validate defaults and documented TOML examples after TOML-to-JSON conversion with the published v1 schema and `Config` -> observed: `published_schema_accepts_defaults_and_documented_configuration` passed
- [x] 3.2 @integration (agent) validate unknown root, memory, MCP-entry, and invalid-bound cases with both layers -> observed: `schema_and_parser_reject_unknown_keys_and_shared_bounds` passed
- [x] 3.3 @integration (agent) run native semantic config tests -> observed: `native_validation_keeps_cross_field_rules_authoritative` passed for command XOR URL, endpoint arguments, and mode-dependent part count

## 4. Published documentation asset

- [x] 4.1 @integration (agent) run the docs build and content checks -> observed: `mise run docs:check` passed VitePress build, formatting/lint, local links, anchors, and public-content validation

## 5. Metadata labels guide later estimates

- [x] 5.1 @eval (agent) inspect serialized metadata fixtures for verified, API-equivalent, configured-assumption, and built-in-assumption records -> observed: core metadata tests asserted pinned/API-equivalent and assumption provenance; unknown/custom records retained absent prices

## 6. Release-quality verification [critical]

- [x] 6.1 @regression (agent) run the full workspace coverage suite -> observed: root completed coverage with exit 0 at 93.46% (38,100/40,764 lines); record: `/private/tmp/kuru-phase1-catalog-coverage.log`
- [x] 6.2 @regression (agent) run workspace lint and tooling checks -> observed: `mise run lint` exited 0 (`/private/tmp/kuru-phase1-catalog-lint.log`) and `mise run lint:tooling` exited 0
- [x] 6.3 @manual (agent) complete an independent scoped review -> observed: review cleared the model-catalog and configuration-schema behavior after the mixed-source bound, route/basis, and map-bound parity corrections
