## 1. Restrict rust-cache saves to main

- [x] 1.1 Add `save-if: ${{ github.ref == 'refs/heads/main' }}` (combined with
      the existing shard condition for the `shard` job's
      `native-coverage-${{ inputs.os }}` key; the `collect` job's own step
      already restores only, `save-if: false`, and needs no change) to the
      `quality-lint`, `quality-typecheck`, `quality-tooling`, `quality-docs`,
      `native-coverage-${{ inputs.os }}`,
      `native-install-${{ inputs.os }}` (renamed from `native-install-windows`
      by main's #110, now covering Linux/macOS/Windows installs),
      `native-build-${{ matrix.target }}` and `native-platform-windows`
      rust-cache steps, and verify
      `actionlint`/`mise run lint:tooling` accepts the workflow YAML.
- [x] 1.2 Update `packages/kuru-delivery/tests/release_workflow.rs`'s exact
      `save-if` string assertion to match the new combined condition; add
      `every_rust_cache_step_restricts_saves_to_main` scanning all three
      workflow files for the same guarantee on every rust-cache step; verify
      `mise run //packages/kuru-delivery:test` passes.

## 2. Pin a prebuilt cargo-llvm-cov

- [x] 2.1 Replace `cargo:cargo-llvm-cov@0.9.1` with
      `aqua:taiki-e/cargo-llvm-cov@0.9.1` in `mise.toml`,
      `packages/kuru-delivery/mise.toml`, the workflow `install_args`,
      `docs/dependencies.md`, and (after #111 merged and replaced
      `support/windows-coverage.ps1` with a unified Rust orchestrator) the
      `LLVM_COV_TOOL` const and its two error-message test assertions in
      `packages/kuru-delivery/src/coverage/orchestrate.rs`; verify
      `mise install` resolves the new pin and `cargo-llvm-cov llvm-cov
      --version` prints `cargo-llvm-cov 0.9.1`.
- [x] 2.2 Refresh `mise.lock` and `packages/kuru-delivery/mise.lock` with
      `mise lock`/`mise -C packages/kuru-delivery lock` so each old `cargo:`
      backend entry is removed and the new `aqua:` entry carries checksums for
      linux-x64, linux-arm64, macos-arm64, macos-x64 and windows-x64; verify
      `git diff --exit-code -- mise.lock packages/kuru-delivery/mise.lock`
      only shows this change.

## 3. Verify

- [x] 3.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling
      ::: cospec:validate ::: cospec:managed:check ::: docs:check` and record
      each exit code.
- [x] 3.2 Run `mise run //packages/kuru-delivery:test` and record the result.
- [x] 3.3 Record the pre-merge `gh api repos/replygirl/kuru/actions/cache/usage`
      total as the before number; the after number is measured post-merge by a
      later orchestrator PR, not here.
