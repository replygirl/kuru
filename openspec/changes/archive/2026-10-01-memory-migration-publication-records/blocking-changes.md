# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Affected surfaces

- `packages/kuru-memory/src/store/migrations.rs`: schema registry, attempt
  build/validate/publish, classification.
- `packages/kuru-memory/src/store/migrations/template_shape.rs`: template
  shape check and authority tables.
- `packages/kuru-memory/src/store/creation_template.rs`: template build
  (consumes the new registry definition; no key-formula change).
- `openspec/specs/versioned-memory/spec.md`: durable spec text.
- `docs/memory.md`, `apps/kuru-docs/concepts/memory.md`, `docs/development.md`.

## Notes

- `openspec/changes/archive/2026-10-01-memory-template-creation` and
  `openspec/changes/archive/2026-09-30-main-pool-migration-classification`
  already shipped: the embedded-template path and main-pool (no
  branch/commit-pool) classification are both live on `main` at
  `CURRENT_VERSION = 7`. This change builds on both without needing either
  reopened.
- No other change under `openspec/changes/` is active at the time this change
  was scaffolded (only this change's own directory exists outside
  `openspec/changes/archive/`), so there is nothing else to sequence against.
- The maintainer brief notes P5 (cold path on one staging engine) is being
  developed in parallel on another branch touching `store/stage_worker.rs`
  and `open_inner`. That branch is not represented in `openspec/changes/` at
  scaffold time; this change is scoped to `migrations.rs`, the shape check,
  the template key inputs and tests specifically so a later rebase stays
  mechanical, per the maintainer's constraint.
