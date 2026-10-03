# Tasks

## 1. Isolate and trace fixture Git

- [x] 1.1 Add `tests/support/fixture_git.rs` with the isolated environment, flags and per-call Trace2 file, and verify both advisory helpers and the scanner fixture use it
- [x] 1.2 Report full Job member image paths in `kuru-platform` and update its Windows tests, and verify with root `lint:windows` (native run in Windows CI only)
- [x] 1.3 Add T1 (no `child_start`), T2 (hostile environment in a child process, with control), T3 (`--show-origin` origins) and T4 (Trace2 tail in the launch-failure panic), and verify they pass locally

## 2. Build fixture repositories once per test binary

- [ ] 2.1 Build advisory fixture templates once per binary and copy them per test, and verify per-test fixture Git spawns are zero

## 3. Verification

- [ ] 3.1 Run delivery advisory tests three times, the delivery and platform suites, format, lint, `lint:windows`, typecheck, `lint:tooling` and `cospec validate --all --strict`, and record exit codes
