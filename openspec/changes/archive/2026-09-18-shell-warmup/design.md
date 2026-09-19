## Context

Root cause (shared with the already-fixed installer flake, PR #44): a cold
stock Windows PowerShell 5.1 engine start — loading
`System.Management.Automation.dll`, `types.ps1xml`/`format.ps1xml`, building
the first runspace — can stall for tens of seconds before any script line
executes, work that `-NoProfile`/`-NonInteractive` do not skip. The
installer fixture already warms this cost for its own path
(`kuru-delivery`'s raw `powershell.exe -Command` bootstrap spawn). This
change addresses the *other* stock-shell path: `packages/kuru-connectors`'s
ToolHost `shell` tool (`tools.rs`'s `shell_inner`), reached for real by
`apps/kuru-tui`'s Windows tests through the compiled `kuru` CLI binary's
`tool shell` subcommand (a third-level child process: test binary → `kuru`
binary → `powershell.exe`). A warm-up that runs inside the *test binary's*
own process does not run in the same OS process as that eventual
`powershell.exe` spawn either way (the CLI binary is a separate process from
the test binary), so the mechanism that actually helps is the same one the
installer fixture already relies on: pre-touching the OS-level engine
cold-start cost (module/DLL disk cache, AV scan, NGEN) shared across
separate `powershell.exe` launches on the same machine — not literal
in-process state reuse.

## Decisions

- **Reuse `ToolHost::execute("shell", …)` itself, not an extracted
  `shell_inner` seam.** The read-only investigation this change implements
  recommended extracting a `pub(crate)` seam out of `shell_inner` so a new
  test-support function could call the exact argument-building code
  directly. `ToolHost::execute("shell", …)` already routes to that same
  code unmodified (confirmed by reading the `"shell" =>` dispatch arm in
  `tools.rs`, which calls the module-private `shell` function that
  `shell_inner` backs on Windows) — it is at least as faithful a "same
  launch configuration, not duplicated" as the seam, and needs zero changes
  to non-test `src/tools.rs`. Given the investigation's own open question
  ("worth confirming [the seam] is intended... since the task frames this
  as a test-flake fix"), avoiding any non-test `src/` edit at all is the
  more conservative choice for a minimal fix, and was chosen instead.
- **A new `shell_warmup.rs` module, not an addition to the existing
  `test_support.rs`.** `test_support.rs` is declared `#[cfg(test)] mod
  test_support;` (private, crate-internal only) because it uses `axum`,
  which is a `[dev-dependencies]`-only crate for `kuru-connectors`.
  `apps/kuru-tui`'s tests need to reach the warm-up as an ordinary
  (feature-gated) dependency, not as `kuru-connectors`'s own `cfg(test)`
  unit-test build — dev-dependencies are unavailable in that mode, so
  widening `test_support.rs`'s gate to include `feature = "test-support"`
  would fail to compile (`axum` unresolved). The warm-up instead lives in
  its own module using only `kuru-connectors`'s ordinary dependencies
  (`tempfile`, `serde_json`, `tokio`, `kuru-core`), gated
  `#[cfg(any(test, feature = "test-support"))]` so it is visible both to the
  crate's own tests and to a downstream `test-support`-featured dev-dependency
  build, without perturbing `test_support.rs` or its `axum`-based fixtures at
  all.
- **Per-process-once guard lives in the shared module, not duplicated per
  test file.** `tokio::sync::OnceCell` inside `kuru-connectors` already
  gives "once per process" for free: each `*.rs` file under
  `apps/kuru-tui/tests/` compiles to its own separate test binary (separate
  OS process), each statically linking its own copy of `kuru-connectors`,
  so the module-level static is naturally scoped per test binary without a
  second, redundant `OnceCell`/`OnceLock` wrapper in each `tests/*.rs` file.
  Each call site is a thin, uncached `fn ensure_powershell_warm()`.
  Originally (as first shipped in this change) it directly spun a
  current-thread Tokio runtime and blocked on it, matching the existing
  `windows_cli.rs:415-418` `Builder::new_current_thread().block_on(...)`
  pattern already used elsewhere in this file for bridging a sync `#[test]`
  into async code. That direct pattern panics ("Cannot start a runtime from
  within a runtime") when the same `Sandbox::new()`/wrapper call happens
  inside a caller that is itself already on a Tokio runtime — e.g. an
  `#[tokio::test]` async test constructing a `Sandbox` synchronously — which
  Windows CI hit for `apps/kuru-tui/tests/cli.rs`. The fix (follow-up on this
  branch, not a new change) moved the runtime bridging into
  `kuru_connectors::shell_warmup::ensure_stock_powershell_warm()`, a shared
  sync wrapper that always runs the warm-up future on a fresh, dedicated OS
  thread with its own `current_thread` runtime (`block_on_dedicated_thread`)
  and joins it — safe whether or not the caller's own thread is already
  inside a Tokio runtime, and regardless of that runtime's flavor. Each test
  file's `ensure_powershell_warm()` now just calls that shared wrapper
  instead of building its own runtime.
- **Warm-up's own PowerShell call requests the shell tool's max
  `timeout_ms` (120 000), wrapped in a distinct outer 130 s bound.** The
  shell tool's own default (30 000 ms) is the exact thing that can be too
  short for a cold engine start (the sibling installer investigation
  observed stalls up to ~100 s); passing an explicit `timeout_ms` argument
  within the tool's already-validated `1..=120_000` range is an ordinary,
  existing capability of the tool (already exercised by other tests with
  custom `timeout_ms` values), not a change to its default or validated
  range. The outer 130 s Tokio timeout is a second, distinctly labelled
  layer purely to bound the warm-up call itself (e.g. against a hang in
  `ToolHost::new` or permission setup outside the tool's own internal
  timeout enforcement) and produces its own message, never the shell tool's
  `"shell timed out"` text, so CI logs stay unambiguous about which layer
  failed.
- **Scope: `cli.rs`, `windows_cli.rs`'s one matching test, and `trust.rs`.**
  `trust.rs` was not named in the investigation's deliverable list, but its
  own `Sandbox` (mirroring `cli.rs`'s) spawns the real `kuru` CLI's `tool
  shell` for several assertions — the investigation's own "Reached by"
  section already lists it as reaching the same code path, same
  third-process shape, just omits it from the later deliverable summary.
  Left out, matching the investigation's stated non-goals: `windows_cli.rs`
  tests exercising raw `powershell.exe` for unrelated purposes (PE/MSVC
  inspection, mise-based source builds/install entrypoints) and
  `packages/kuru-connectors`'s own unit/integration tests — neither set was
  named as flaky, and adding warm-ups pre-emptively there is out of scope
  for a minimal fix (mirrors why the sibling installer change also left
  `kuru-delivery`'s `bootstrap_windows.rs`/`windows_update.rs` untouched).

## Risks / Trade-offs

- The OS-level warm effect (not same-process) is inherently probabilistic,
  not a guarantee — this mirrors the already-shipped installer warm-up's own
  accepted trade-off, not a new risk this change introduces.
- Adding `kuru-connectors` as an `apps/kuru-tui` dev-dependency with
  `features = ["test-support"]` unions that feature into every test target
  in the `kuru` package for test builds (Cargo feature unification is
  per-package, not per-target). The feature only compiles in additional
  `#[cfg]`-gated test-only code (verified: `cargo check -p kuru
  --all-targets --all-features` and `cargo test -p kuru` both pass; no
  runtime-affecting code is behind this feature).

## Operational surface

CI-only change: affects the Windows native-tests job's `apps/kuru-tui`
shard (`ci.yml`). No bind address, container, secret, or binary-version
change. The three touched test binaries (`cli`, `windows_cli`, `trust`) are
already built and run by the existing `//apps/kuru-tui:test` mise task on
that job; no new CI step or job is added.
