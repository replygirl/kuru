## 1. Engine warm-up ahead of the timed install step

- [x] 1.1 Add a Windows-gated `warm_up_powershell_engine` helper and
  `ENGINE_WARM_UP_TIMEOUT` constant in `apps/kuru-tui/tests/embedded_runtime.rs`,
  using the exact same launch configuration as the install command
  (`env_clear` + the same explicit env set, `-NoProfile -NonInteractive`,
  never setting `PSModulePath`) to run a trivial inline script; call it from
  `install_packaged` before building the install command. Verified by
  reading the diff: `COMMAND_TIMEOUT` and the install command's own
  construction are byte-for-byte unchanged, and the new call sits strictly
  before the install `Command::new(bootstrap...)` is built.
- [x] 1.2 Verify the non-Windows (`#[cfg(unix)]`) `install_packaged`/`execute`
  path is untouched and the crate still builds and its non-Windows tests
  still pass on this host. Verified: the real acceptance test
  `packaged_install_and_update_preserve_complete_offline_memory` (which
  exercises `install_packaged`/`execute` end to end on this macOS host)
  passed — `test packaged_install_and_update_preserve_complete_offline_memory
  ... ok` (39.84s).

## 2. Timeout attribution sampling

- [x] 2.1 Add `NativeChild::duplicate_diagnostic_handle`,
  `ProcessSample`, and `sample_process` (wrapping `GetProcessTimes` /
  `GetProcessMemoryInfo`, later switched to the kernel32-exported
  `K32GetProcessMemoryInfo` to keep shipping imports off `psapi.dll`; see
  PR #44) to `packages/kuru-platform/src/windows/process.rs`,
  refactoring `duplicate_process` onto a shared
  `duplicate_process_with_access` helper with no behavior change for
  existing callers (`duplicate_process_handle`,
  `duplicate_inherited_process_handle`, `current_process_handle`). Verified:
  `cargo check --target x86_64-pc-windows-msvc -p kuru-platform` is clean,
  and every existing caller still resolves to the same
  `PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION` mask.
- [x] 2.2 In `packages/kuru-delivery/src/command.rs`'s
  `output_with_limit_and_timeout`, spawn a diagnostic sampler against a
  duplicated handle while the timed future is in flight, `abort()` it
  unconditionally as soon as that future settles (success or failure), and
  append its collected samples to the existing timeout error text next to
  `stdout_eof`/`stderr_eof`. Verified by reading the diff: the success-path
  return value and every existing field in the error format string are
  unchanged, and a sample failure is recorded as text (`sample_error=...`),
  never propagated as an error.
- [x] 2.3 Verify by type-checking `kuru-delivery` for
  `x86_64-pc-windows-msvc` -> could not complete on this host:
  `reqwest`'s `aws-lc-sys` build script requires `<windows.h>` from the
  Windows SDK, unavailable when cross-compiling from macOS (reproduced
  identically with and without `--no-default-features`, so this is a
  pre-existing host limitation, not something this change introduced — see
  verification.md 1.3). The changed crate itself (`kuru-platform`) does
  type-check for the real target (2.1). This package's regular test suite
  on this host (`mise run //packages/kuru-delivery:test -- command`) ran
  clean — 0 Windows-gated tests selected/run, as expected on macOS; no
  non-Windows target or test regressed.

## 3. Checks

- [x] 3.1 Run `mise run format:check`, the owning packages' lint tasks, and
  `mise run typecheck`; record results. All clean: `format:check` passed
  (after one `cargo fmt --all` to apply this change's own formatting);
  `//apps/kuru-tui:lint`, `//packages/kuru-delivery:lint`,
  `//packages/kuru-platform:lint` (`clippy -D warnings`, all targets/features)
  all passed; `mise run typecheck` passed across every workspace member.
- [x] 3.2 Run `mise run cospec -- validate windows-installer-warmup-and-timeout-attribution --strict`
  and resolve or explicitly acknowledge any reported soft blockers before
  archiving. Passed: `0 errors, 0 warnings — validation passed`. `mise run
  cospec:validate` (workspace-wide) also passed.
