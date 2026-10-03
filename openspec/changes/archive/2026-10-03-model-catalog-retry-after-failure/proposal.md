# Proposal

## Why

`Harness::selected_model_metadata` (`packages/kuru-runtime/src/engine.rs`)
filled its `tokio::sync::OnceCell<Vec<ModelInfo>>` with
`get_or_init(|| async { self.provider.models().await.unwrap_or_default() })`.
One transient `models()` failure therefore cached an empty catalog for the
whole harness lifetime, with no reset path and no record that anything had
failed. From then on the selected model's advertised efforts, default effort,
prices and context window silently degraded to the embedded snapshot (for known
IDs, through `enrich_model`) or to the assumed context window, even after the
provider recovered. Found by the fixed-wait audit of 2026-10-02, §3B rank 5
(unit U7).

## What Changes

- The cell is filled with `get_or_try_init(|| self.provider.models())`. A
  failed listing leaves the cell empty, so the next metadata lookup lists
  again; the first successful listing is cached exactly as before (an empty
  `Ok` listing is still cached and is not a failure).
- Retry cadence: a failed listing records a retry instant
  `MODEL_CATALOG_RETRY_AFTER` (30 s) ahead, beside the cache. Until then every
  lookup uses the fallback without calling `models()`, including lookups that
  were queued on the cell behind the failed attempt. tokio's `OnceCell`
  releases its permit on an initializer error and the next waiter runs the
  initializer itself; without the gate, actor asks fanned out with `join_all`
  would each run a slowly failing listing in turn, the N-th waiting up to N
  times the connector's 60 s `IO_TIMEOUT` before its completion started, on
  every turn. With the gate a provider that stays down costs at most one
  listing attempt (bounded by that 60 s HTTP timeout) per 30 s window, and
  concurrent lookups wait at most for that one attempt, as they did when the
  failure was cached.
- The failure is not propagated. The failing call proceeds with an empty
  catalog, so the existing not-found fallback yields the embedded snapshot or
  the assumed context window and completions keep working.
- The failure is logged with `tracing::warn!` on target `kuru.runtime`,
  message `model catalog unavailable`, fields `operation = "model-catalog"`,
  `status = "unavailable"`, `model` and `error` (the error rendered with
  `{:#}`). The diagnostics ring's existing field allow-list persists only the
  operation and status; model and provider error text are not written there.
- Warning cadence: once per harness, guarded by an `AtomicBool`
  (`model_catalog_warned`) beside the cache. This is the cache's own lifetime;
  a session switch inside one harness resets neither. A provider that stays
  down for many turns therefore leaves one record rather than one per turn,
  while every lookup still retries the listing.
- Test-only: a process-wide `kuru.runtime` warning recorder in the
  kuru-runtime test binary, installed once behind a `OnceLock` with
  `set_global_default` and `rebuild_interest_cache`, mirroring kuru-memory's
  `RetainedStageRecorder` pattern that the `clippy.toml` ban points to.
  Records are scoped by the `model` field. No dev-dependency is added.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The `model-catalog` spec already requires live discovery to remain the
source of availability and unknown models to remain usable; the implementation
wrongly let one failed discovery stand in for discovery for the rest of the
session.

## Impact

- `packages/kuru-runtime/src/engine.rs`: `Harness` gains
  `model_catalog_warned: AtomicBool` and
  `model_catalog_retry_at: Mutex<Option<Instant>>`; `selected_model_metadata`
  uses `get_or_try_init`, skips listing inside the retry window and warns
  once; a new `model_catalog_tests` test module with a fake provider whose
  `models()` fails a set number of times (optionally after a delay) then
  succeeds, and the process-wide warning recorder.
- No public API, configuration, protocol, dependency or Cargo.lock change. No
  user documentation describes catalog-failure behavior; docs/usage.md's
  diagnostics description (no remote error text in the ring) stays true.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
