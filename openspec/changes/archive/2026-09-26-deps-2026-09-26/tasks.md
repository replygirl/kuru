# Tasks

## 1. Rust dependencies

- [x] 1.1 Pin `clap` 4.6.7, `rustix` 1.1.5, `tiktoken-rs` 0.12.1 and `jsonschema` 0.58.0 and update only those Cargo.lock entries, and verify the Cargo.lock diff contains no unrelated package movement
  - Evidence: `cargo update -p clap -p rustix -p tiktoken-rs -p jsonschema` moved clap/clap_builder/clap_derive to 4.6.7, jsonschema/jsonschema-regex/jsonschema-value/referencing to 0.58.0, rustix to 1.1.5 and tiktoken-rs to 0.12.1, and removed the now-unused `fancy-regex` 0.17.0 duplicate; no other package moved. `jsonschema` 0.58's removed `unsatisfiable_pointers` API is unused; `mise run typecheck` compiled every target (exit 0, 150s).
- [x] 1.2 Confirm the `tiktoken-rs` 0.12.1 embedded `o200k_base.tiktoken` asset hash and license are unchanged and update the connector third-party notice, and verify the observed SHA-256 matches the documented hash
  - Evidence: the 0.12.1 registry source's `assets/o200k_base.tiktoken` SHA-256 is `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`, identical to 0.12.0 and the documented OpenAI hash; its Cargo.toml license remains MIT and the upstream `v0.12.1` tag exists.

## 2. Tooling

- [x] 2.1 Pin mr-boxington 1.18.0 and cospec 0.8.2, regenerate cospec-managed files with `cospec update`, and verify `mise run cospec:managed:check` reports no drift and `mise run cospec:validate` passes
  - Evidence: `cospec update` regenerated the managed harness files, schema templates and manifest; `mise run cospec:managed:check` reported "no drift" and `mise run cospec:validate` passed 43/43 specs with 0 errors and 0 warnings. mbx 1.18.0 installed; the root `MBX_TARGET_VIEWS=0` keeps the worktree `target/` a real directory (observed).
- [x] 2.2 Retarget the cospec compatibility preload to the embedded OpenSpec 1.13.1 bundle, and verify the standalone `cospec_contract` test fails without it and passes with it
  - Evidence: with cospec 0.8.2 and the unchanged 1.11.0-keyed preload, `cospec_contract` failed ("could not parse JSON from: openspec instructions apply"). Keyed to `openspec-1.13.1-0d2dfc42f009c982` (entry SHA-256 `0d2dfc42f009c9827c4a035cccf0ad8ceec6d0670a9fa78094e1939cb25830c1`), it passed 1/1. The removal condition remains unmet.
- [x] 2.3 Pin Communiqué 1.4.2, verify every supported platform's lock entry through the documented per-platform provenance commands, and invert the native-adapter thinking canary, verifying the delivery `release_notes` tests pass
  - Evidence: the four documented `MISE_OS/MISE_ARCH` lock commands (run with CI's mise 2026.9.4) recorded `provenance_verified = true` for linux-x64, linux-arm64, macos-arm64 and windows-x64, keeping the gnu Linux and x86_64 Windows assets. The canary `actual_native_anthropic_adapter_still_rejects_claude_five_thinking` failed under 1.4.2 (exit 0, one `/v1/messages` call, notes written); it is now `actual_native_anthropic_adapter_accepts_claude_five_thinking` and `release_notes` passed 13/13.
- [x] 2.4 Pin Node 26.10.0, npm 12.1.0 with its tarball SHA-512, and the docs app's npm dependencies, refresh package-lock.json, and verify the docs build and content checks pass
  - Evidence: the downloaded npm 12.1.0 tarball SHA-512 matches the registry integrity; npm 12.1.0 declares Node `>=26.0.0`; Node 26.10.0 bundles npm 11.19.1. `npm install --package-lock-only` moved only oxfmt, oxlint and vitepress-plugin-llms entries. `mise run docs:check` exit 0 (9s): build, oxfmt, oxlint and content/link checks passed.
- [x] 2.5 Refresh the five-platform mise locks at root, docs and delivery scopes, and verify `mise install` succeeds with no remaining lock drift
  - Evidence: locks were refreshed with the documented five-platform commands using the CI-pinned mise 2026.9.4 (local 2026.9.13 omits `provenance_verified`), plus the existing musl/baseline variant platforms at root and the per-platform cospec provenance lines; root retains 5 `provenance_verified` and 9 signer entries, as before. The delivery lock gains the `cargo-llvm-cov` 0.9.1 task-tool entry that both mise versions produce. Locked 2026.9.4 installs and ordinary 2026.9.13 `mise install`/`mise run` left all three lockfiles byte-identical.

## 3. Documentation and verification

- [x] 3.1 Update the dependency audit, development, release and notes-configuration text for the new pins, and verify no stale previous-version references remain outside historical records
  - Evidence: a repository search for the previous versions finds only historical statements (tokenizer addition, Communiqué 1.3.5 limitation) and synthetic repository-validation fixtures.
- [x] 3.2 Run format, lint, typecheck, tooling lint, docs check and the ordinary test suite, and verify each passes with observed results recorded here
  - Evidence: `mise run format:check` exit 0 (8s; rerun after edits 5s); `mise run lint` exit 0 (65s); `mise run typecheck` exit 0 (150s); `mise run lint:tooling` exit 0 (6s, repository metadata invariants passed); `mise run docs:check` exit 0 (9s); `mise run test` exit 0 in 864s after the canary change (70 test binaries, 1392 passed, 0 failed, 4 ignored). The first `mise run test` run failed in 127s on the canary. Coverage was not run locally; the pre-push hook and CI run it.
