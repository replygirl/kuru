## Context

The dock's known-cost branches (`dock_meters`, `render.rs:870-878`) are
reachable only when `EstimateFold` folds an `InvocationUsage` whose
`price_at_invocation` is `Some`. That price comes from
`ModelCatalog::enrich(route, model)`, keyed on `(route, model_id)`. Every
real-PTY fixture drives its turn through a locally mocked HTTP server, whose
`api_base` is never the literal string `https://api.openai.com/v1` that
`kuru-runtime::selected_model_metadata` requires for `ModelRoute::OpenAiResponses`;
every fixture therefore classifies as `ModelRoute::CustomResponses`, and the
embedded catalog has no priced record for that route — confirmed by
`ModelCatalog::from_json_checked`'s own validation, which only accepts a
price on `(CodexSubscription, ApiEquivalent)` or `(OpenAiResponses,
ApiStandard)`. This is the root cause of every existing real-PTY fixture
being stuck on `"cost unknown"`.

## Goals / Non-Goals

**Goals:**
- Reach a real, spawned-binary frame with a known cost, without spoofing DNS
  or TLS for the real `api.openai.com` host.
- Keep the relaxation reachable only from test code, invisible to an
  ordinary build.

**Non-Goals:**
- No change to `EstimateFold`/`SessionFold` accounting math.
- No new shipping config key.
- No change to `ModelRoute` classification in `kuru-runtime`.

## Decisions

- **Layer a fixture catalog onto the embedded one inside
  `ModelCatalog::embedded()`, gated by `#[cfg(feature = "test-support")]` and
  `KURU_TEST_MODEL_CATALOG_PATH`.** Rejected alternative: point the fixture's
  `api_base` at the literal `https://api.openai.com/v1` string and intercept
  the real DNS/TLS route to that host — technically reaches
  `ModelRoute::OpenAiResponses` without touching the catalog, but requires
  faking name resolution and terminating TLS for a real hostname inside the
  test process, and risks a real network attempt if the redirect ever fails.
- **Accept `(CustomResponses, ApiStandard)` as a price pairing, but only from
  the test-support override path, not from `from_json`'s public
  validation.** `CustomResponses` is the only route a locally mocked provider
  can ever produce without also changing `kuru-runtime`'s route
  classification (out of scope — `selected_model_metadata`'s exact-string
  match is the only thing that decides `OpenAiResponses`, and no file the
  task scoped this change to includes `engine.rs`). The alternative — widen
  the *shared* check in `from_json` — was rejected: it would let a real,
  shipped catalog record price an arbitrary custom endpoint, which the
  existing check deliberately forbids (a custom `api_base` can point at any
  model; the catalog cannot know what is actually running behind it). Instead
  `from_json_checked` takes a bool the public `from_json` always passes as
  `false`; only the new `#[cfg(feature = "test-support")]`
  `from_json_test_override` passes `true`. `from_json`'s observable behavior
  is therefore unchanged for every caller outside this seam.
- **Merge by extending `records`, re-checking for duplicate `(route,
  model_id)` keys across the merge**, rather than writing a second parser —
  reuses the exact same deserialization and per-record validation
  (size/citation/price format) as the embedded catalog; the only difference
  is the one relaxed `ensure!`.

## Operational surface

No bind address, container/runner topology or secret is introduced. The
"interactive" surface here is the existing `kuru` terminal binary spawned as
a local child process under a real PTY, exactly as every other
`apps/kuru-tui/tests/terminal.rs` fixture already does; the only new runtime
input is the test-only `KURU_TEST_MODEL_CATALOG_PATH` env var, read from the
child's own environment, never a network or IPC surface. Binary/arch: the
test spawns `env!("CARGO_BIN_EXE_kuru")`, the same host-target debug build
already used by the rest of the suite.

## Risks / Trade-offs

- [Risk] A future real catalog record could accidentally collide with the
  fixture's `("custom-responses", "fixture")` key. → The merge step
  explicitly re-checks for duplicate keys across the merge and fails closed;
  the fixture id is chosen to be obviously test-only.
- [Risk] The relaxed validation path could be reached outside tests if
  `test-support` were ever enabled in a release build. → `test-support` is
  not part of any `default` feature set and is only enabled by
  `apps/kuru-tui`'s own `test-support` feature, which mirrors the existing
  `kuru-platform`/`kuru-memory` pattern already used exclusively by
  `mise run test` tasks.
