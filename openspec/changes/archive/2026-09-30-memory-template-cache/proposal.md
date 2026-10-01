# Proposal

## Why

Creating a project today always pays the full schema-migration chain on its
own staging engine: four engine starts before the restart-removal PR lands,
and the chain runs again for every new project on the machine. The
per-machine store template design
(`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`) lets a
future project instead copy a pre-migrated template's `data/` and adopt it on
one engine start, but nothing can build, verify or quarantine that template
yet — PR #145 (`memory-template-stage-adoption`, archived) landed only the
receiving side (the identity marker, bootstrap adoption, the shared
template-shape check, and the two copy-remnant recovery classes), with no
template for it to receive.

This change (P4b-i of that design) lands the template cache itself — its
on-disk layout, its content key, the single-engine build that produces it,
the verified copy primitive, structural and shape verification, identity-bound
quarantine on a positive verdict, and the rule that no open path ever garbage
collects another key's template or any lock file — plus the CI test-fixture
warm-up that builds one template per job, on every OS and under coverage,
before any fixture's first fresh open. No product open selects this path yet:
`Creation::Default` and `Creation::Cold` are unchanged, and every start count a
product opener observes today stays the same. Landing the cache and its
warm-up first, with every guard and the class-guard counters proven in CI,
lets the next change (P4b-ii) wire `Creation::Default` to copy from a template
already known to build, verify and quarantine correctly, rather than debugging
both at once.

## What Changes

- New production module `packages/kuru-memory/src/store/creation_template.rs`
  (or an equivalent path under `store/`) with `ensure_in(root, …)` (verify,
  or wait under the exclusive key lock and build) and `create_in(root, …)`,
  taking the template root as an explicit parameter so product code passes
  `<cache>/<DOLT_VERSION>/templates` and tests can pass a private root through
  a new crate-private `OpenOptions` field, matching `test_support/template.rs`'s
  existing `instantiate_in(root, …)` shape.
- Layout under `templates/`: a permanent `<key>.lock`, a published
  `<key>/manifest.json` + `<key>/data/`, transient `.build-<key>-<uuid>/`,
  `.stage-<key>-<uuid>/` and `.rejected-<key>-<uuid>/` directories, and (Windows
  only) a `lifecycles/` root for build-store lifecycle locks kept outside the
  moved tree.
- The key: a `sha256` over a domain tag and format constant, the pinned engine
  version/digest/target triple, the schema and usage-schema versions and every
  compiled migration-registry definition's digest, and the creation SQL
  statements, placeholder literals, commit-message formats and `server.yaml`
  behavior block, refactored into `const` arrays so the key is complete.
  Supervisor bytes and source digests are excluded. Computed once per process.
- The build: a new stage-worker job (`TemplateBuild`) that runs bootstrap with
  the placeholder identity, `initialize`, the full migration chain,
  `upgrade_usage` and `validate_usage` on the usage branch, `validate_active`
  and the existing shared template-shape assertion (`store/migrations/template_shape.rs`)
  on one engine, guarded by the exclusive key lock (itself the build's reap
  guard), with a non-waiting sweep of abandoned `.build-*`/`.stage-*` entries
  for that key first.
- Capture, byte scan, manifest and publication: promoting the capture walk and
  classification of `test_support/template.rs` to production, a byte scan of
  the captured tree refusing the build store's path, its secrets and the host
  name, and publication by a no-replace rename with bounded retry, leaving a
  verified stage for the caller on rename failure.
- Verification on each use: a structural check (manifest parse, format, key,
  top-level names) and a per-file size-and-SHA-256 `copy_into` that refuses
  links and extra hard links and compares the entry set — both as tested
  library functions; no product open calls them yet.
- Identity-bound quarantine on a positive verdict only (structural, copy-byte,
  adoption or shape), through `CreationFailure::{TemplateVerdict, Engine, Io}`,
  best-effort and bound to the judged directory's identity.
- No garbage collection on any open path: no open removes another key's
  template, another key's `.rejected-*`, or any lock file; lock files are
  permanent.
- Test-fixture warm-up: a second `OnceCell` in `warm_runtime_cache` that builds
  or verifies the template once per job, bounded, waiting for a peer process's
  build; `temporary()`/`temporary_cold()`/`open_temporary()` warm first; a
  crate-private per-fixture `Fixture::{Unwarmed, Warmed}` token on
  `OpenOptions` set by `warmed_open_options`/`OpenOptions::warmed()`; a
  deterministic guard in `open_inner` failing an unwarmed fresh writable open
  against the shared test root at once; a fallible `cache_dir()` plus
  `warmed_cache_dir()` for spawned kuru-tui owners; warm-up held outside every
  `spawn_gate::spawning()` guard, audited at every enclosing site; a `prefetch`
  step; coverage warming in the instrumented process.
- Class guards in `engine_ledger` (a template built inside `open_inner`, or
  quarantined under the shared root) failing the fixture that did either,
  unless opted in.
- Spec delta on `versioned-memory` limited to the cache (layout, key, build
  assertions, verification, quarantine, no garbage collection) plus the two
  sentences deferred from PR #145: a classifier refusal during the shape check
  is engine-side evidence, not a verdict, and an older client that cannot parse
  the typed rejection fails closed with a parse error.
- Docs: `docs/development.md` (shared test cache, warm-up, prefetch, how to
  reclaim it), `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` (the
  template cache beside the engine cache, what it holds and does not, that a
  project purge does not touch it, and that no open uses it until the next
  change).

**Not in this change (P4b-ii and later):** `Creation::Default` routing to the
template, the copy worker inside `open_inner`, cold fallbacks on a template
failure, restart removal (P5), records (P6), a reclamation command, and any
change to product timeouts, deadlines or retries — the only new waits are in
fixture warm-up, outside every product deadline.

## Capabilities

### New Capabilities

None. The template cache extends the existing durable-memory capability; see
Modified Capabilities.

### Modified Capabilities

- `versioned-memory`: adds the per-machine store template cache (layout, key,
  build, verification, quarantine, no garbage collection) as a requirement
  alongside the existing "Template-born stage identity and adoption"
  requirement this capability already carries from PR #145.

## Impact

- New: `packages/kuru-memory/src/store/creation_template.rs` (or sibling
  module), its unit/integration tests, and a template-root field on
  `OpenOptions` under `cfg(any(test, feature = "test-support"))`.
- Changed: `packages/kuru-memory/src/store/stage_worker.rs` (new
  `TemplateBuild` job), `packages/kuru-memory/src/store/migrations/template_shape.rs`
  (reused, not changed in shape), `packages/kuru-memory/src/provision.rs`
  (cache layout awareness), `packages/kuru-memory/src/store.rs` (`open_inner`'s
  deterministic fixture guard; `temporary()`, `temporary_cold()`,
  `open_temporary()` warm first), `packages/kuru-memory/src/test_support.rs`
  (`warm_runtime_cache`, `open_options` → `warmed_open_options`, fallible
  `cache_dir()`/`warmed_cache_dir()`, `spawn_gate` interaction),
  `packages/kuru-memory/mise.toml` (`prefetch`), every fixture call site of
  `test_support::open_options` across kuru-memory, kuru-runtime and kuru-tui
  (switch to the warmed form), and `apps/kuru-tui/tests/support/{memory,directories,trust}.rs`
  plus `apps/kuru-tui/src/ui/runtime_tests.rs` if it runs inside a Tokio
  runtime.
- Docs: `docs/development.md`, `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`.
- No schema version change, no new environment variable, no change to the
  active-path product start counts, no BREAKING change: every existing opener
  observes identical behavior, because no open path yet reads the template.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
