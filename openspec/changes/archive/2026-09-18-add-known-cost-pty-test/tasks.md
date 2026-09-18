## 1. Test-support catalog override seam

- [x] 1.1 Add `test-support` feature to `packages/kuru-core/Cargo.toml` and verify `cargo check -p kuru-core --all-features` compiles clean.
- [x] 1.2 Add `KURU_TEST_MODEL_CATALOG_PATH`-driven catalog layering to `ModelCatalog::embedded()` in `packages/kuru-core/src/model_catalog.rs`, gated by `#[cfg(feature = "test-support")]`, reusing `from_json`'s validation via a new `allow_custom_responses_price` parameter that the public `from_json` always passes as `false`; verify with the existing `cargo test -p kuru-core --all-features` suite (`malformed_catalogs_are_rejected_before_enrichment` and the rest of `tests/model_catalog.rs` unchanged).
- [x] 1.3 Propagate `kuru-core/test-support` alongside `kuru-platform/test-support` in `apps/kuru-tui/Cargo.toml`'s own `test-support` feature; verify with `cargo check -p kuru --all-targets --all-features`.

## 2. Priced fixture and real-PTY test

- [x] 2.1 Add `apps/kuru-tui/tests/fixtures/priced-model-catalog.json` (one `custom-responses` + `fixture` record, `api-standard` basis) and verify `cargo test -p kuru --test terminal --all-features -- real_pty_status_bar_renders_known_cost_from_priced_invocation --exact` passes.
- [x] 2.2 Add the regression test's mock handler (`priced_complete`, reporting `input_tokens`, `output_tokens`, zero `cached_tokens` and zero `reasoning_tokens` so the fold leaves no term unapplied) and its `/cost`-line token parser to `apps/kuru-tui/tests/terminal.rs`; verify the dock row renders `"≈$X est"` (not `"≥$X est"`) and `/cost` states the identical figure in the same frame, at both 120 and 80 columns.
- [x] 2.3 Verify the regression signature directly: revert `packages/kuru-core/{Cargo.toml,src/model_catalog.rs}` and `apps/kuru-tui/Cargo.toml` only (keep the test and fixture), confirm the new test FAILS with `cost unknown`, then restore and confirm it PASSES again.

## 3. Whole-suite and gate checks

- [x] 3.1 `cargo fmt --all -- --check` (`mise run format:check`) -> pass.
- [x] 3.2 `cargo clippy -p kuru --all-targets --all-features --locked -- -D warnings` (`mise run //apps/kuru-tui:lint`) -> pass.
- [x] 3.3 Full `cargo test -p kuru --test terminal --all-features --no-fail-fast` -> 18 passed, 0 failed, 1 pre-existing ignored; no regression in the existing cost-unknown real-PTY tests.
