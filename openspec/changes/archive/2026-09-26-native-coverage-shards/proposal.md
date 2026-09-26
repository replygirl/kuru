## Why

The Windows `memory-runtime` coverage shard is the critical path of every
native CI run (it runs two long real-memory suites serially, and has reached
the 90-minute job ceiling), and the arm64 Linux `native-build` job recompiles
every dependency cold on each run. Splitting that shard and caching the build
shortens CI without changing any product behavior.

## What Changes

- `packages/kuru-delivery/src/coverage.rs` (CI coverage tooling behind the
  `tooling` feature, not shipped in the `kuru` executable): `SHARDS` becomes
  five entries, `memory` (`kuru-memory`) and `runtime` (`kuru-runtime`)
  replacing `memory-runtime`.
- `.github/workflows/native-tests.yml`: the Windows shard matrix mirrors the
  five `SHARDS` rows; the report job downloads five shard artifacts into
  `inputs/<shard>/`; the shards share one rust-cache key
  `native-coverage-windows`, saved only by `connectors-core-platform`.
- `.github/workflows/ci.yml` `native-build`: rust-cache with
  `shared-key: native-build-${{ matrix.target }}`, a private `runner.temp`
  bundle directory selected before any build, and `CARGO_INCREMENTAL: "0"`.
- `packages/kuru-delivery/tests/release_workflow.rs`: pin five shards, five
  downloads, the single shared key, and require the workflow matrix rows to
  equal `SHARDS` exactly.
- `docs/development.md`: shard names and count; `native-build` caching.
- Not included: caching the instrumented Windows coverage target (see
  `tasks.md` 2.1 for the reason it is deferred).

## Impact

Jobs: `native-tests (windows-2025)` gains one shard job and one download step;
`native-build` gains a cache restore/save and a bundle-directory step. Runner
labels, timeouts, the Unix coverage job, secrets and the required checks
(`ci-gate`, `Lint PR title`) are unchanged. The Windows shard artifact names
`…-coverage-windows-memory-attempt-<n>` and `…-runtime-attempt-<n>` replace
`…-memory-runtime-attempt-<n>`; only the report job in the same run consumes
them.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
