## Why

The repository accumulated Python helpers and a root JavaScript/Python tooling
project, conflicting with the user's explicit architecture priorities:
apps/packages first, mise orchestration second, Rust third, other tooling only
where it adds necessary value. Cospec's standalone binary embeds OpenSpec;
the root npm dependency and Bun are not required for its workflow.

## What Changes

- Move installer, packaging, repository validation and release helpers into a
  Rust package under packages/kuru-delivery, with package-owned mise tasks.
- Replace Python terminal/protocol subprocess fixtures with Rust equivalents.
- Make the updater use the native installer library and preserve verification,
  atomic replacement and malformed-archive rejection.
- Remove Python/uv/Bun/root JS manifests and localize Node/npm dependencies to
  apps/kuru-docs. Keep Cargo workspace metadata only for Rust compilation and
  dependency resolution; mise owns monorepo task discovery and aggregation.

## Impact

Development/build tasks, native delivery package, updater implementation,
test fixtures, docs app dependency ownership and CI/release invocation paths.
CLI behavior, psychological-framework behavior, private memory boundaries,
terminal interactions and installer safety guarantees remain invariant.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
