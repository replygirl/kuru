## Why

Windows CI intermittently fails `apps/kuru-tui/tests/cli.rs`'s
`cli_file_crud_and_shell_require_real_capabilities` (main@a1eac2b, run
35408171182): `tool execution failed: shell timed out; stderr: <pending
EOF>`, with the fixture's own source entry/completion markers both
`Err(NotFound)` — i.e. `powershell.exe` never reached the first line of the
CLI's `tool shell` script within the shell tool's own `timeout_ms` budget
(product default 30 000 ms). This is the same PowerShell 5.1 engine/host
cold-start stall already diagnosed and fixed for the installer fixture in
`apps/kuru-tui/tests/embedded_runtime.rs` (PR #44, `warm_up_powershell_engine`)
— loading `System.Management.Automation.dll`, `types.ps1xml`/`format.ps1xml`,
and building the initial runspace, work that happens before any script line
runs and that `-NoProfile`/`-NonInteractive` do not skip — but reached
through a different code path: the ToolHost stock shell in
`packages/kuru-connectors` (`tools.rs`'s `shell_inner`), not
`kuru-delivery`'s install/update bootstrap script. The installer fixture's
warm-up launches raw `powershell.exe -Command` with different flags and
without the ToolHost's module-import source wrapping, so it does not warm
this path; no shared warm-up exists for the ToolHost stock-shell tests.

## What Changes

Add one shared, Windows-only stock-PowerShell engine warm-up
(`kuru_connectors::shell_warmup::warm_up_stock_powershell_engine`) that
drives a trivial no-op command through the real
`ToolHost::execute("shell", …)` path — the exact executable resolution,
flags, reconstructed `PSModulePath` policy, and module bootstrap a real
`tool shell` call uses — rather than re-deriving those launch flags in
test-only code. It is guarded by a process-wide `tokio::sync::OnceCell` (safe
under concurrent test threads) and has its own distinct, generous outer
bound (130 s) with a message clearly labelled as the warm-up, separate from
the shell tool's own `timeout_ms` (which stays at its existing default and
validated `1..=120_000` range — unchanged). `apps/kuru-tui`'s Windows tests
that exercise the ToolHost stock shell (`cli.rs`'s `Sandbox::new`, the one
`windows_cli.rs` test that spawns the real CLI's `tool shell`, and
`trust.rs`'s `Sandbox::new`, which also spawns real `tool shell` calls) call
it once per test-binary process before their first shell invocation. Non-
Windows builds are unaffected: the shared helper no-ops off-Windows, and
every call site is itself `#[cfg(windows)]`-gated (or, for `windows_cli.rs`,
already behind that file's `#![cfg(windows)]`).

Implementation note (deviation from the initial investigation plan): the
plan's primary recommendation was to extract a `pub(crate)` seam out of
`shell_inner` in `packages/kuru-connectors/src/tools.rs` so a new function in
the existing `test_support.rs` could call it directly. Two things make that
both unnecessary and worse: (1) `ToolHost::execute("shell", …)` already
routes to `shell_inner` unmodified — reusing the real public entry point is
at least as faithful a "reuse, not duplicate" as reusing an internal
function, and needs zero changes to non-test `src/tools.rs`; (2)
`test_support.rs` is unconditionally `#[cfg(test)]`-only because it uses
`axum` (a `[dev-dependencies]`-only crate) — widening its gate to also cover
`feature = "test-support"` (needed for `apps/kuru-tui`'s dev-dependency to see
it) would fail to compile, since dev-dependencies are not available when a
crate is built as an ordinary (non-test) dependency of another crate. The
warm-up therefore lives in its own new module,
`packages/kuru-connectors/src/shell_warmup.rs`, gated
`#[cfg(any(test, feature = "test-support"))]` and using only ordinary
(non-dev) dependencies already available to `kuru-connectors`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — this corrects test-harness flakiness; no capability spec was wrong.

## Impact

- `packages/kuru-connectors/src/shell_warmup.rs` (new): the shared warm-up.
- `packages/kuru-connectors/src/lib.rs`: declares
  `#[cfg(any(test, feature = "test-support"))] pub mod shell_warmup;`.
- `apps/kuru-tui/Cargo.toml`: adds `kuru-connectors` under
  `[dev-dependencies]` with `features = ["test-support"]` (Cargo unions this
  with the existing ordinary dependency for test builds).
- `apps/kuru-tui/tests/cli.rs`, `apps/kuru-tui/tests/trust.rs`: each gets a
  `#[cfg(windows)] fn ensure_powershell_warm()` wrapper, called from
  `Sandbox::new()`.
- `apps/kuru-tui/tests/windows_cli.rs`: gets the same wrapper (file is
  already `#![cfg(windows)]`), called at the top of
  `built_in_shell_reconstructs_stock_module_paths_without_losing_other_environment`
  — the one test in this file the investigation confirmed spawns the real
  CLI's `tool shell` (third-process, same shape as `cli.rs`).
- No change to `packages/kuru-connectors/src/tools.rs` (the shell tool's
  timeout default/validation, `ShellFailureCategory`, or `shell_inner` are
  untouched), and no change to `packages/kuru-delivery`'s unrelated
  bootstrap warm-up/sampler.
- Left untouched, and named here so the gap is visible rather than silent:
  `apps/kuru-tui/tests/windows_cli.rs`'s `pe_inspection_...`,
  `source_update_builds_through_mise_...`, and
  `source_install_entrypoint_...` tests, which spawn raw `powershell.exe`
  for unrelated purposes (MSVC/mise tooling, install-entrypoint scripts) and
  were not named as flaky by the investigation; and
  `packages/kuru-connectors/tests/windows_commands.rs` /
  `packages/kuru-connectors/src/tools.rs` unit tests, not named as flaky
  either. Adding warm-ups there is out of scope for this minimal fix.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
