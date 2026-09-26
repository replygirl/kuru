## 1. Restrict rust-cache saves to main

- [x] 1.1 Add `save-if: ${{ github.ref == 'refs/heads/main' }}` (combined with
      the existing shard condition for `native-coverage-windows`) to the
      `quality-lint`, `quality-typecheck`, `quality-tooling`, `quality-docs`,
      `native-coverage`, `native-coverage-windows`, `native-install-windows`
      and `native-build-${{ matrix.target }}` rust-cache steps, and verify
      `actionlint`/`mise run lint:tooling` accepts the workflow YAML.
- [x] 1.2 Update `packages/kuru-delivery/tests/release_workflow.rs`'s exact
      `save-if` string assertion to match the new combined condition and
      verify `mise run //packages/kuru-delivery:test` passes.

## 2. Pin a prebuilt cargo-llvm-cov

- [x] 2.1 Replace `cargo:cargo-llvm-cov@0.9.1` with
      `aqua:taiki-e/cargo-llvm-cov@0.9.1` in `mise.toml`,
      `packages/kuru-delivery/mise.toml`,
      `packages/kuru-delivery/support/windows-coverage.ps1`, the workflow
      `install_args`, and `docs/dependencies.md`; verify `mise install`
      resolves the new pin and `cargo-llvm-cov llvm-cov --version` prints
      `cargo-llvm-cov 0.9.1`.
- [x] 2.2 Refresh `mise.lock` with `mise lock` so the old `cargo:` backend
      entry is removed and the new `aqua:` entry carries checksums for
      linux-x64, linux-arm64, macos-arm64, macos-x64 and windows-x64; verify
      `git diff --exit-code -- mise.lock` only shows this change.

## 3. Verify

- [x] 3.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling
      ::: cospec:validate ::: cospec:managed:check ::: docs:check` and record
      each exit code.
- [x] 3.2 Run `mise run //packages/kuru-delivery:test` and record the result.
- [x] 3.3 Record the pre-merge `gh api repos/replygirl/kuru/actions/cache/usage`
      total as the before number; the after number is measured post-merge by a
      later orchestrator PR, not here.
