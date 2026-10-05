# Tasks

## 1. Silent-owner CLI fixture

- [x] 1.1 Switch only the publication-failure test from `Sandbox::fresh_cache()` to `Sandbox::new()` and verify the fresh project, real Dolt, `WRITE_FAILURE_ENV=1`, success/demo JSON and exact `OPENING` assertions are preserved; verify the separate cold-start fixture and all production bytes remain unchanged.
- [x] 1.2 Run the exact failing CLI test and independent cold CLI progress fixture through the TUI mise task with verified offline bundle inputs and the package-prepared supervisor; run memory's focused failed-publisher and readiness-bound controls through its owning task and record actual outcomes and cleanup failures without claiming a historical negative reproduction.
- [x] 1.3 Run relevant formatting, TUI lint/typecheck, docs build/content checks, strict Cospec validation and managed drift checks; record source review and actual results before a normal-hook intermediate checkpoint and authorized publication.
- [ ] 1.4 Require fresh exact-head native instrumented CI, including Windows x64/ARM, before completing hosted acceptance; do not substitute local macOS results or mark archive readiness while hosted checks are pending.
- [ ] 1.5 Complete the evidence ledger and final strict/apply validation once all acceptance passes. Mandatory closeout after that administrative task: execute the actual Cospec archive, confirm the active record is absent and archive present, record actual archive output, and make the final normal-hook commit before root-reviewed publication and fresh final CI. No forced archive or manual move.

## Evidence

Acceptance authored before implementation. Base: `1ac1a1751b28636771ade55893024850cc2f312a`; original CI failure: `37244540171`, job `111559868142`, CLI 7 passed/1 failed. Readiness expired after 30014 ms with no activity and owner still running; a later cleanup expiry observed an endpoint record. The engine-delay mechanism and exact cleanup timing remain unknown. Production intentionally retains its one-window silent-owner readiness bound.

Actual strict validation passed (0 errors/warnings) and apply exited 0 clear before implementation. Root independently read the three artifacts and exact one-line source delta and accepted its scope/source. `git diff --numstat` confirms the only source change is one addition/one deletion in this test; all assertions and the separate cold fixture are byte unchanged.

Local macOS checks completed so far, through owning mise tasks with `KURU_MBX=0`, `nofile=4096` for real memory, and verified offline inputs:

- Exact publication-failure CLI: exit 0, 1 passed/0 failed, 9.31 s test time (158.52 s task including initial preparation/compilation).
- Independent cold CLI progress/owner exit/reopen: exit 0, 1 passed/0 failed, 15.80 s test time (48.30 s task). Observed cold=14.500229333 s, close=91.374542 ms, reopen=1.024985291 s; these are fixture observations, not a performance claim.
- `mise run docs:check`: exit 0, 10.30 s aggregate. Its actual task headers include docs build (2.07 s), lint, format/check, and content check (3.67 s; public artifacts, local links and anchors passed); no adjacent-task-name shortcut was used.
- `mise run format:code`: exit 0, 2.85 s. `mise run cospec:managed:check`: exit 0, no drift. `git diff --check`: exit 0.
- TUI host lint and typecheck: exit 0, 83.35 s and 34.36 s tasks respectively.
- Exact memory failed-publisher control: exit 0, 1 passed/0 failed, 5.10 s test time (87.75 s task including initial memory-test compilation).
- Memory `service::tests::progress_wait::` controls: exit 0, 7 passed/0 failed, 1.29 s test time (9.26 s task), covering advancing/stalled/exited/retired/failing/stale records and unconditional stand-in release.
- Exact memory readiness deadline/client-phase split control: exit 0, 1 passed/0 failed, 1.02 s test time (3.93 s task), preserving a live silent owner's one-window readiness failure.
- TUI Windows-target lint: exit 0, 102.81 s task, using its package-owned stand-in build tools and the actual x64 Windows archive; this is compilation/lint evidence, not native Windows execution.

No owner cleanup errors occurred in these focused controls. Host archive SHA-256 `ad1e3770accbb7e8a059069228ead22bdf6f2759131212451b44f518661b8e40` and Windows x64 archive SHA-256 `55e9c2053496d29a1c1b23e6ad689779da41518f8cc257f531c5b20f37621c5d` were checked against the manifest and imported by the package task offline; the Windows input is 40082076 bytes. The owning tasks rebuilt their missing outputs, completed prefetch and used `KURU_TEST_SUPERVISOR_PREPARED=1`. Source patch SHA-256: `6dcf6f5652d408e36e9909c00910aa1cef1be86785a8ae1986ce1669adf8c42d`.

Normal-hook intermediate checkpoint and authorized publication follow this completed local ledger. Native instrumented CI and final archive/commit remain pending. No historical negative reproduction is claimed; the separate cleanup-classification ambiguity is unmodified.
