# Tasks

## 1. Silent-owner CLI fixture

- [x] 1.1 Switch only the publication-failure test from `Sandbox::fresh_cache()` to `Sandbox::new()` and verify the fresh project, real Dolt, `WRITE_FAILURE_ENV=1`, success/demo JSON and exact `OPENING` assertions are preserved; verify the separate cold-start fixture and all production bytes remain unchanged.
- [x] 1.2 Run the exact failing CLI test and independent cold CLI progress fixture through the TUI mise task with verified offline bundle inputs and the package-prepared supervisor; run memory's focused failed-publisher and readiness-bound controls through its owning task and record actual outcomes and cleanup failures without claiming a historical negative reproduction.
- [x] 1.3 Run relevant formatting, TUI lint/typecheck, docs build/content checks, strict Cospec validation and managed drift checks; record source review and actual results before a normal-hook intermediate checkpoint and authorized publication.
- [x] 1.4 Require fresh exact-head native CI through its existing topology, including instrumented Linux/macOS/Windows x64 checks and Windows ARM behavioral checks, before completing hosted acceptance; do not substitute local macOS results or mark archive readiness while hosted checks are pending.
- [x] 1.5 Complete the evidence ledger and final strict/apply validation once all acceptance passes. Mandatory closeout after that administrative task: execute the actual Cospec archive, confirm the active record is absent and archive present, record actual archive output, and make the final normal-hook commit before root-reviewed publication and fresh final CI. No forced archive or manual move.

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

Normal-hook intermediate checkpoint `2b1c7559fb22e79701b97d25169b1a0e41ab2973` completed with exit 0; formatting, strict Cospec validation and conventional-message hooks passed. Root reviewed that exact clean checkpoint, published PR225 with normal pre-push hooks, and verified its exact head. An earlier automatic approval rejection happened before any push process started; root subsequently supplied verified direct user authorization and a fresh review approved its normal push. No hook bypass or rejected-action retry was performed by this implementation agent.

Fresh exact-checkpoint hosted CI [37247569925](https://github.com/replygirl/kuru/actions/runs/37247569925) completed SUCCESS at `2b1c7559fb22e79701b97d25169b1a0e41ab2973`: all 66 jobs terminal, 62 success and four normal conditional engine-topology skips, no failure or cancellation. Linux/macOS/Windows x64 instrumented partitions and their coverage merges/gates passed; Windows ARM used the workflow's existing uninstrumented behavioral partitions/merge and passed. Native installation, offline runtime, previous-release update, platform checks, ARM Linux memory checks, static/docs, open-time and final CI gates also passed. The four skipped jobs are three non-ARM `dolt-windows-arm64` jobs and the unused reproducible Windows ARM engine-build route; the selected pin-verified ARM input passed. This is the configured native topology, not a claim of Windows ARM instrumentation.

Authoritative full hosted receipt is preserved in root ignored `tmp/roadmap/pr225-checkpoint-ci-success-2026-10-04.json`; local closeout preserves a bounded exact-head/job summary. This implementation agent parsed the saved receipt and independently asserted the exact SHA, completed success conclusion, 66 terminal jobs and success/skipped-only outcomes. This is checkpoint acceptance; root-owned publication and fresh final-head CI remain required after actual archive/final commit. No historical negative reproduction is claimed; the separate cleanup-classification ambiguity is unmodified.

Task 1.5's evidence ledger and administrative validation completed after all acceptance passed: `mise run cospec -- validate silent-owner-fixture-cache --strict` exited 0 with zero errors/warnings, and `mise run cospec -- apply silent-owner-fixture-cache --json` exited 0 with a clear gate, no blockers or warnings. All three returned context files were read in full. The initial sandboxed validation failed before Cospec could run because mise could not write its existing cache; the authorized escalated invocation passed. Logs `17-hosted-closeout-strict.log`, `17b-hosted-closeout-strict.log` and `18-hosted-closeout-apply.json/.stderr` preserve that distinction. After checking the administrative task, prearchive strict validation again exited 0 with zero errors/warnings, and apply exited 0 clear with all five tasks complete; every returned context was read (`19-prearchive-strict.log`, `20-prearchive-apply.json/.stderr`).

## Mandatory closeout

Actual `mise run cospec -- archive silent-owner-fixture-cache` exited 0. Its stdout was:

```text
Archived: silent-owner-fixture-cache (ci) → openspec/changes/archive/2026-10-04-silent-owner-fixture-cache/
Specs:    skipped
```

The active directory is absent and the archived directory contains the expected four files. No archive warning, spec merge or sibling blocker update was printed. The typed CI record intentionally has `skip_specs: true`; no durable spec change was needed. Raw stdout/stderr are retained in ignored `tmp/roadmap/21-archive.stdout/.stderr`.

Postarchive `mise run cospec -- validate --all --strict` exited 0 with zero errors/warnings for its active-change/durable-spec selection (zero active changes, 43 specs); existing long-requirement INFO notices remain in the log. `mise run cospec:managed:check` exited 0 with no drift. `git diff --check` passed, and source comparison to the exact checkpoint is empty. Logs are `22-postarchive-all-strict.log` and `23-postarchive-managed.log`. No optional scan of all historical archived records was repeated, and no unrelated record or durable spec was edited.

The normal-hook final commit remains mandatory immediately after postarchive checks and before root-reviewed final publication. Its actual SHA, hook outcome and clean status are reported in the ignored finalization receipt after execution, avoiding a self-referential commit SHA in this ledger. Root owns final publication, fresh final-head CI and protected merge; checkpoint CI success is not a claim that those later actions have happened. The one-line reviewed source patch remains frozen exactly at the checkpoint.
