## Why

Model discovery currently exposes only provider-supplied IDs, display names, and
effort labels. Kuru consequently cannot truthfully describe a model's known
limits or API-equivalent rates, and its configuration format has no published,
machine-readable contract for editors and tooling.

Phase 1 needs an offline, route-aware source of metadata that enriches live
discovery without substituting for it. It also needs a versioned schema that
matches Kuru's strict configuration parser so unsupported authority-affecting
keys fail clearly instead of being silently accepted.

## What Changes

- Add sourced, optional model limits, price components, and capabilities to the
  core discovery contract, preserving provider-advertised IDs and effort
  strings.
- Embed and validate a bounded route-and-model keyed catalog of pinned official
  metadata, including provenance and API-equivalent subscription price bases.
- Enrich connector model discovery with route-appropriate snapshot facts while
  retaining live availability as the source of selectable models.
- Add a fallback-only, configurable assumed context-window setting. An unknown
  model remains selectable; its resolved window is explicitly labelled as an
  assumption.
- Publish configuration JSON Schema v1 in the docs public assets and document
  its relationship to authoritative TOML parsing and semantic validation.

## Capabilities

### New Capabilities
- `model-catalog`: Route-aware, sourced offline metadata enrichment for live
  model discovery and honest unknown-model fallback limits.
- `configuration-schema`: Versioned public JSON Schema for Kuru configuration
  with strict unknown-key and parser-parity guarantees.

### Modified Capabilities

None.

## Impact

- `packages/kuru-core`: public model metadata and catalog APIs, configuration,
  embedded snapshot asset, tests, and any test-only schema validator dependency.
- `packages/kuru-connectors`: route-specific live-discovery enrichment while
  preserving unknown IDs and efforts.
- Runtime, TUI, and test `ModelInfo` constructors require mechanical metadata
  defaults.
- `apps/kuru-docs/public` and configuration documentation gain the v1 schema
  asset and reference.
- No migration, live catalog fetch, account-availability claim, usage ledger,
  inference gate, or subscription billing calculation is introduced.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
