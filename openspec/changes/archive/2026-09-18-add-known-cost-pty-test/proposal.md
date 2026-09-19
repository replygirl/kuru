## Why

Phase 1's standing dock row (`dock_meters` in `apps/kuru-tui/src/ui/render.rs`)
has two known-cost formatting arms — a complete `"≈$X est"` estimate and an
incomplete `"≥$X est"` subtotal — but every real-PTY fixture in
`apps/kuru-tui/tests/terminal.rs` drives its turn through a mock server whose
`api_base` is never the literal `https://api.openai.com/v1`, so the runtime's
route classification always resolves to `ModelRoute::CustomResponses`, a
route the embedded model catalog never prices. Every existing real-PTY
assertion about cost is therefore stuck on the `"cost unknown"` branch; the
two known-cost arms are covered only by narrower unit tests elsewhere, never
by a real spawned `kuru` process rendering an actual frame. A regression that
broke known-cost rendering specifically (a wrong decimal format, a missing
`$`, a swapped `≈`/`≥` glyph) would pass every current real-PTY check.

## What Changes

Add a `test-support`-gated seam to `kuru-core`'s model catalog
(`ModelCatalog::embedded()`) that, when `KURU_TEST_MODEL_CATALOG_PATH` names
a fixture catalog file, layers its records on top of the embedded catalog.
The seam is inert unless the `test-support` Cargo feature is compiled in and
the env var is set; it never changes what an ordinary build accepts. Reusing
it requires one narrow, equally test-support-gated relaxation of the
catalog's route/price-basis check (`custom-responses` + `api-standard` is
now accepted from this seam alone), because a locally mocked provider always
classifies as `ModelRoute::CustomResponses` — the literal
`https://api.openai.com/v1` match is unreachable from a test fixture without
spoofing DNS/TLS for that real hostname, which is out of scope. `from_json`'s
own public validation is unchanged for every other caller.

A new fixture catalog (`apps/kuru-tui/tests/fixtures/priced-model-catalog.json`)
gives the mock `responses` provider's `fixture` model a real price. A new
real-PTY test drives one turn against it and asserts the standing dock row
renders the complete `"≈$X est"` token, that `/cost` states the identical
figure in the same frame, and that the figure is derived from the actual
cumulative token totals `/cost` reports (not an assumed invocation count or
a hardcoded dollar string).

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-core/Cargo.toml`: new `test-support` feature (no
  dependencies gated behind it).
- `packages/kuru-core/src/model_catalog.rs`: new
  `embedded_uncached`/`layer_test_override`/`from_json_test_override`
  functions, all `#[cfg(feature = "test-support")]`; `from_json`'s validation
  body is factored into `from_json_checked` with an added, still-false-by-
  default `allow_custom_responses_price` parameter. No change to `from_json`'s
  observable behavior.
- `apps/kuru-tui/Cargo.toml`: propagate `kuru-core/test-support` alongside
  the existing `kuru-platform/test-support` in the crate's own `test-support`
  feature.
- `apps/kuru-tui/tests/fixtures/priced-model-catalog.json`: new fixture.
- `apps/kuru-tui/tests/terminal.rs`: new test
  `real_pty_status_bar_renders_known_cost_from_priced_invocation` plus its
  mock handler and a small `/cost` token-count parser shared by no other
  test.

## Surfaces

- [x] interactive — asserts rendered TUI dock-row and `/cost` output.
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
