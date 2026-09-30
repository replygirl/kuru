# Proposal

## Why

Creating a project today always pays the full schema-migration chain on its own
staging engine. The per-machine store template design
(`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`) lets a future
project be created by copying a pre-migrated template's `data/` into an
ordinary stage and starting one engine there instead, which the design's
measurements put at roughly half the engine starts of today's cold path once a
template is warm. Nothing can select that path safely yet: a copied stage
carries a placeholder identity shared by every template copy, and nothing
rewrites it, verifies it, or recovers a stage a crash interrupts mid-adoption.

This change (P4a of that design) lands the receiving side of that protocol
before anything produces a template: the identity marker that names the
template a stage came from, one-shot adoption in the Dolt bootstrap that gives
a copy its own instance identity and credentials under a typed verdict against
the compiled placeholder, the two recovery classes a crash can leave behind in
a copied stage, and the template-shape check the build and the stage engine
will both need later. Landing it now, with no caller, lets P4a's ten tests
exercise adoption and recovery directly against hand-built template-like
stages (per `engine-contract-findings.md` S1/S2, already measured against real
Dolt 2.3.5) before the template cache or the creation selector exist, so a
defect here is caught in isolation rather than inside the larger P4b change.

## What Changes

- `identity.json` gains an optional `template: <key>` field (opaque string;
  the real key computation is P4b's) with a `stage_template_key` reader in
  `server.rs`, exposed to `store.rs` without a second struct. Cold stores keep
  byte-identical `identity.json` records: the field is
  `#[serde(default, skip_serializing_if = "Option::is_none")]`, so nothing is
  written when it is absent, and `Identity` keeps `deny_unknown_fields`, so an
  older binary fails closed on a template-born store.
- A one-shot adoption group in the Dolt bootstrap (`initialize_database`,
  `server.rs`), gated on `!identity.initialized && identity.template.is_some()`:
  under the supervisor's own compiled placeholder-key comparison, it verifies
  the placeholder instance/scope row on the usage branch and on `main`, then in
  one connection rewrites that row to the new instance and scope and commits on
  each ref in turn (usage branch first), before the existing `kuru_reader`
  creation and the `initialized: true` rewrite. It never runs for a cold store
  and is never retried or run from recovery.
- A new supervisor response, `Response::TemplateRejected(String)`, distinct
  from `Response::Failed` and from client-side startup errors, sent only when
  a comparison *completed* and returned a value other than the compiled
  placeholder or the expected affected-row count. A key mismatch between
  `identity.template` and the supervisor's own compiled key (a different,
  non-bytes-related failure) and every engine, SQL, I/O or deadline failure
  stay `Response::Failed`; the stage is preserved either way, never retried.
- Two new `recover_staging` classes, checked before any class that starts an
  engine: Class R (copy remnant: no `identity.json`, `data/` present, no other
  top-level entries) and Class U (unready template stage: `identity.json` with
  `template` set, no `ready.json`). Both wait for quiescence within the
  existing bound and then preserve the stage under `interrupted/` without
  starting an engine.
- One shared template-shape-check function, parameterized by the expected
  identity row and the registry-derived branch/commit-count expectations, for
  later reuse by both the template build (P4b) and this PR's own adoption path
  on the stage engine.
- Ten new tests (`store/template_stage_tests.rs`, `server/template_identity_tests.rs`
  and the shape module's unit test) exercising adoption, the typed verdict,
  and classes R and U against hand-built template-like stages (a template
  store built by the real chain under the build identity, its stopped `data/`
  copied by the checked fixture copy, and the copy's identity record written
  last), using a per-directory engine start count in the engine ledger to
  prove zero engine starts for classes R and U.
- `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` gain a short,
  accurate passage on template-born stores' identity and recovery, stated as
  having no user-visible effect yet, since no open takes this path.

No caller selects the template path in this change: `Creation::Default` and
`Creation::Cold` behave exactly as today, and no template cache, key
computation, build worker, copy worker or creation selector is added (P4b).

## Capabilities

### New Capabilities

(none — this extends the existing versioned-memory capability)

### Modified Capabilities

- `versioned-memory`: adds the template identity marker and its
  backward-compatible serialization, one-shot placeholder adoption with a
  typed verdict distinct from engine/SQL/I/O failures, and the two new
  unreachable-engine-start recovery classes for a copied template stage.

## Impact

- `packages/kuru-memory/src/server.rs`: `Identity` struct (new optional
  `template` field), `initialize_database` (adoption group, key comparison),
  `Response` enum (`TemplateRejected` variant), `supervisor_request` (error
  downcast to the new verdict type), new `pub(crate) fn stage_template_key`.
- `packages/kuru-memory/src/store.rs`: `recover_staging` (classes R and U,
  ordered before the existing identity-without-marker writable-recovery
  branch at store.rs:7443-7454), `preserve_unready_stage` reuse.
- `packages/kuru-memory/src/store/migrations/template_shape.rs` (a child of
  `migrations.rs`): the template-shape-check function, parameterized so the
  P4b build and this PR's S1 adoption path share one implementation.
- `packages/kuru-memory/src/store/stage_worker.rs`: the `adopt_and_mark` job
  that the creation path will call for a copied stage.
- `packages/kuru-memory/src/store/template_stage_tests.rs`: the adoption,
  verdict, shape and class R/U tests, plus a fail-closed test for stages
  that stay unrecognized.
- `packages/kuru-memory/src/store/engine_contract_tests.rs`: unchanged, but
  referenced — it already exercises adoption SQL by hand and its S1/S2
  findings ground this design's bootstrap protocol.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`: short passage, no
  user-visible behavior change stated.
- `openspec/specs/versioned-memory/spec.md`: requirement deltas described
  below.
- No schema version change, no new migration step, no change to
  `CURRENT_VERSION` or timeouts/deadlines/retries.
- Not BREAKING: no existing open path changes behavior; the new field is
  additive and optional, and older binaries already fail closed on any
  `deny_unknown_fields` record they cannot parse, which is the intended and
  documented behavior for a template-born store.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
