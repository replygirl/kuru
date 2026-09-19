## 1. Shared warm-up compiles and is reusable [critical]

- [x] 1.1 @unit (agent) `cargo check -p kuru-connectors --all-targets --all-features --locked`, `mise run //packages/kuru-connectors:typecheck`, `:lint` -> all pass; new `shell_warmup.rs` module and widened `lib.rs` gate compile clean under clippy `-D warnings`.
- [x] 1.2 @unit (agent) `cargo test -p kuru-connectors --all-targets --all-features --locked` -> 199 passed, 0 failed; existing suite unaffected, new module's own `#[cfg(all(test, windows))]` test does not run on macOS as expected.

## 2. Consuming crate resolves and compiles [critical]

- [x] 2.1 @unit (agent) `cargo check -p kuru --all-targets --all-features --locked` -> passes; `kuru_connectors::shell_warmup` is visible from `apps/kuru-tui`'s dev-dependency edge via feature unification.
- [x] 2.2 @unit (agent) `mise run //apps/kuru-tui:lint` (clippy `-D warnings`, `--all-targets --all-features`) -> passes.
- [x] 2.3 @unit (agent) `mise run format:check` -> passes (rustfmt + taplo, whole repo).

## 3. Touched macOS test binaries still pass [critical]

- [x] 3.1 @integration (agent) `mise run //apps/kuru-tui:test -- cli` -> `tests/cli.rs`: 12 passed incl. `cli_file_crud_and_shell_require_real_capabilities`; `tests/trust.rs`: 5 passed; `tests/windows_cli.rs`: 0 tests (whole file `#![cfg(windows)]`), all 0 failed.
- [x] 3.2 @unit (agent) confirm `ensure_powershell_warm()` in `cli.rs`/`trust.rs` is `#[cfg(windows)]`-gated -> confirmed by source read; the macOS pass in 3.1 is itself the evidence it is a no-op there.

## 4. Regression — the actual named CI failure [critical]

- [~] 4.1 @regression (agent) reproduce `cli_file_crud_and_shell_require_real_capabilities` failing with `tool execution failed: shell timed out; stderr: <pending EOF>` and `Err(NotFound)` source markers before this change, then passing after, on a real Windows runner across repeated runs -> defer: intermittent (~9% of shard runs per the sibling installer-flake investigation's frequency data for the same cold-start mechanism) Windows-only stall; cannot be reproduced or disproven on this macOS host, and no Windows CI run of this exact change has occurred yet — the next Windows CI run of `//apps/kuru-tui:test` is the real signal.
- [x] 4.2 @unit (agent) confirm the shell tool's own `timeout_ms` default (30 000) and validated range (`1..=120_000`) in `packages/kuru-connectors/src/tools.rs` are unchanged -> confirmed: `git diff --stat origin/main` shows that file untouched.

## 5. Non-goals untouched

- [x] 5.1 @unit (agent) confirm `packages/kuru-delivery/src/command.rs` and `apps/kuru-tui/tests/embedded_runtime.rs` (the sibling installer warm-up/sampler) are unchanged -> confirmed: `git diff --stat origin/main` shows neither file touched.
