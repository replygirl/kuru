## 1. Five Windows coverage shards and a cached native build

- [x] 1.1 Split `SHARDS` in `packages/kuru-delivery/src/coverage.rs` into five entries (`memory`, `runtime` separate) and adapt tests that named `memory-runtime`, and verify with `mise run //packages/kuru-delivery:test`
- [x] 1.2 Mirror the five shards in the `native-tests.yml` Windows matrix and report downloads, share one `native-coverage-windows` rust-cache key saved only by `connectors-core-platform`, and verify with actionlint via `mise run lint:tooling`
- [x] 1.3 Add rust-cache, a `runner.temp` bundle directory before any build, and `CARGO_INCREMENTAL: "0"` to `ci.yml` `native-build`, and verify with actionlint and the rust-cache v2.9.2 cleanup source
- [x] 1.4 Update `release_workflow.rs` for five shards, five downloads, the shared key, and an exact matrix-equals-`SHARDS` assertion, and verify with `mise run //packages/kuru-delivery:test`
- [x] 1.5 Update `docs/development.md` shard names/count and `native-build` caching, and verify with `format:code` and `docs:check`
- [x] 1.6 Validate the change record and verify with `cospec:validate` and `cospec:managed:check`

## 2. Deferred follow-on

- [x] 2.1 Decide whether to cache the instrumented Windows coverage target in this change, and verify against the rust-cache v2.9.2 `workspaces` and cleanup source

Evidence (macOS arm64, 2026-09-26; artifacts authored, `validate --strict` passed and `apply --json` exited 0 with gate `clear` before any edit): `mise run //packages/kuru-delivery:test` exited 0 and `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check` exited 0 (details in `verification.md`). `docs/verification.md` keeps its dated 2026-09-15 record (four shards) unchanged. Unrun: `mise run coverage` and the full workspace suite (CI-only for this change), the Windows-only delivery tests, and every hosted job; per-shard wall-clock, `native-build` before/after and rust-cache save/hit evidence come from this PR's CI run.

2.1 decision: instrumented-target caching is not included. In rust-cache v2.9.2, `src/config.ts` resolves a `workspaces` entry as `path.join(root, target)`, so an absolute `${{ runner.temp }}` target becomes a path nested under the checkout; registering it would need a checkout-relative spelling that depends on the hosted runner's directory layout. `src/cleanup.ts` `cleanTargetDir` removes only the *files* of a non-profile subdirectory, so every saved instrumented target would retain an empty `kuru-shard-state/`, which the specified acceptance rule (accept a pre-existing target only without `kuru-shard-state`) rejects on every restore. Making it useful needs a pre-save cleanup step or a different acceptance rule, beyond the scoped change; it remains a follow-on.
