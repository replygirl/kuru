## Why

`apps/kuru-tui/tests/terminal.rs`'s
`real_pty_status_bar_renders_known_cost_from_priced_invocation` depends on
kuru-core's `test-support` feature (the fixture catalog seam reached through
`KURU_TEST_MODEL_CATALOG_PATH`, see `packages/kuru-core/src/model_catalog.rs`).
Without that feature — `cargo test -p kuru --test terminal`, run with no
`--all-features` — the test still runs and FAILS with "not applied: invocation
price" instead of being skipped. This is a red test, not a real defect: it
only fails because its required seam was never wired in for the plain test
build, so anyone running the crate's tests without `--all-features` sees a
false failure.

## What Changes

`apps/kuru-tui/Cargo.toml` gains a `kuru-core` dev-dependency with the
`test-support` feature enabled. Dev-dependency features are only unified into
a package's build during test/bench compilation, so this activates the
`KURU_TEST_MODEL_CATALOG_PATH` fixture seam for every `kuru` test binary
without adding `test-support` to the shipping `kuru` binary's resolved
feature set. `cargo test -p kuru --test terminal` (no `--all-features`) now
passes deterministically instead of failing on a seam it never had.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `apps/kuru-tui/Cargo.toml`: adds `kuru-core = { workspace = true, features = ["test-support"] }` to `[dev-dependencies]`.
- No production code, spec, or shipping-binary feature set changes. Verified via `cargo tree -e features -p kuru --no-dev-dependencies` showing zero `test-support` occurrences.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
