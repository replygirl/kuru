## 1. Shared warm-up helper

- [x] 1.1 Add `packages/kuru-connectors/src/shell_warmup.rs` with
      `warm_up_stock_powershell_engine()`, reusing `ToolHost::execute("shell",
      …)` (not a re-derived copy of `shell_inner`'s launch flags), a
      process-wide `tokio::sync::OnceCell` guard, and its own distinct 130 s
      bound/message — and verify `cargo check -p kuru-connectors --all-targets
      --all-features --locked` passes on macOS (non-Windows branch compiles;
      Windows branch is behind `cfg(windows)`).
- [x] 1.2 Gate the new module `#[cfg(any(test, feature = "test-support"))]
      pub mod shell_warmup;` in `packages/kuru-connectors/src/lib.rs` without
      touching `test_support.rs` (which stays `#[cfg(test)]`-only because of
      its `axum` dev-dependency) — verify `cargo check -p kuru-connectors
      --all-targets --all-features --locked` still passes.
- [x] 1.3 Add `kuru-connectors = { workspace = true, features =
      ["test-support"] }` under `apps/kuru-tui`'s `[dev-dependencies]` — verify
      `cargo check -p kuru --all-targets --all-features --locked` resolves and
      `kuru_connectors::shell_warmup` is visible from `apps/kuru-tui/tests/*.rs`.

## 2. Wire the failing and sibling tests

- [x] 2.1 Add a `#[cfg(windows)] fn ensure_powershell_warm()` wrapper to
      `apps/kuru-tui/tests/cli.rs`, call it from `Sandbox::new()` — verify
      `cli_file_crud_and_shell_require_real_capabilities` and the rest of
      `cli.rs` still pass on macOS (`cargo test -p kuru --test cli`), and that
      the new call site is `#[cfg(windows)]`-gated so it is a no-op elsewhere
      (confirmed by the macOS pass itself: `ensure_powershell_warm` never
      executes there).
- [x] 2.2 Add the same wrapper to `apps/kuru-tui/tests/windows_cli.rs` (file
      is already `#![cfg(windows)]`), call it at the top of
      `built_in_shell_reconstructs_stock_module_paths_without_losing_other_environment`
      — verify the file still reports "0 tests" on macOS (whole file cfg'd
      out, so nothing regresses there); Windows execution stays pending (no
      Windows runner on this host).
- [x] 2.3 Add the same wrapper to `apps/kuru-tui/tests/trust.rs`, call it from
      its own `Sandbox::new()` — verify `cargo test -p kuru --test trust`
      passes on macOS.

## 3. Non-goals confirmed untouched

- [x] 3.1 Confirm the shell tool's own `timeout_ms` default (30 000) and
      validated range (`1..=120_000`) in `packages/kuru-connectors/src/tools.rs`
      are unchanged (no diff to that file).
- [x] 3.2 Confirm `packages/kuru-delivery`'s bootstrap warm-up/sampler and
      `apps/kuru-tui/tests/embedded_runtime.rs`'s existing
      `warm_up_powershell_engine` are unchanged (no diff to either file).

## 4. Checks

- [x] 4.1 Run `mise run format:check`, `//packages/kuru-connectors:lint`,
      `//apps/kuru-tui:lint`, and `mise run typecheck`; record results. All
      clean: `format:check` passed; both lint tasks (`clippy -D warnings`,
      all targets/features) passed; `mise run typecheck` passed across the
      whole workspace.
- [x] 4.2 Run `mise run cospec -- validate shell-warmup --strict` and resolve
      or explicitly acknowledge any reported soft blockers before archiving.
      Passed: `0 errors, 0 warnings — validation passed`.

Regression evidence for the actual named CI failure
(`cli_file_crud_and_shell_require_real_capabilities` timing out on Windows)
is recorded, honestly deferred to the next Windows CI run, in
`verification.md` §4 — it is an intermittent (~9% of shard runs) Windows-only
PowerShell cold-start stall that cannot be reproduced, forced, or disproven
on this macOS host, so no task here claims to have exercised it.
