# Tasks

## 1. Isolate and trace fixture Git

- [x] 1.1 Add `tests/support/fixture_git.rs` with the isolated environment, flags and per-call Trace2 file, and verify both advisory helpers and the scanner fixture use it
- [x] 1.2 Report full Job member image paths in `kuru-platform` and update its Windows tests, and verify with root `lint:windows` (native run in Windows CI only)
- [x] 1.3 Add T1 (no `child_start`), T2 (hostile environment in a child process, with control), T3 (`--show-origin` origins) and T4 (Trace2 tail in the launch-failure panic), and verify they pass locally

## 2. Build fixture repositories once per test binary

- [x] 2.1 Build advisory fixture templates once per binary and copy them per test, and verify per-test fixture Git spawns are zero
  - Observed with a logging `git` shim on PATH, one test per process. The advisory tests that use fixture repositories make 0 per-test fixture Git calls. The exception is the changed-head case: the scanner fixture's own commit is the behavior under test. The template builds make 15 calls in the unit binary and 9 in the integration binary.

## 3. Verification

- [x] 3.1 Run delivery advisory tests three times, the delivery and platform suites, format, lint, `lint:windows`, typecheck, `lint:tooling` and `cospec validate --all --strict`, and record exit codes
  - Local macOS runs, all exit 0: advisory lib and integration tests three times each, `//packages/kuru-delivery:test` (full suite), `//packages/kuru-platform:test`, `format:check`, `lint`, `lint:windows`, `typecheck` and `lint:tooling`.
  - Not run locally: the Windows tests (`kuru-platform` `tests/windows_process.rs`, kuru-delivery `tests/windows_command_capture.rs`) and the advisory tests on Windows. They run natively only in Windows CI; `lint:windows` compiles them.
