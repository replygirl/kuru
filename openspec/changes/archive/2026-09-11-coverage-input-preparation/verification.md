Current acceptance is complete at a1547ed; see final integration below and
native-windows verification, "Final native acceptance: a1547ed". Earlier failed
or pending runs remain historical evidence.

## 1. Cold instrumented execution [critical]

- [x] 1.1 @integration (agent) Resolve the actual pinned mise coverage and ordinary-test graphs -> twelve mise 2026.9.4 graph/info/dry-run queries passed. Coverage emits two delivery archive-preparation commands and one llvm-cov command, without an ordinary memory build/prefetch; its snapshot opt-in remains explicitly unset even when inherited as 1. Ordinary app/memory/runtime tests retain shared prefetch and snapshot=1. Windows-target dry-run was on macOS, not native execution. Evidence: `/tmp/kuru-coverage-input-graph-review.md` and `/tmp/kuru-coverage-input-graphs/`.
- [x] 1.2 @runtime (agent) Run the normal pre-push on macOS arm64 with a new private KURU_DOLT_CACHE -> all eight concurrent hooks passed at 1943c4e, with 14,328/14,757 instrumented lines (97.092905%). The cache was absent before the hook and afterward contained the actual Dolt executable and licenses. Coverage's resolved preparation executed only the two archive-input commands before its own instrumented suite; no ordinary memory prefetch or supervisor snapshot ran. Actual install/update, cold startup, durable SQL/reopen and runtime cases passed. Archive preparation took 1.19/1.57 seconds, the suite 260.25 seconds and coverage including preparation 261.90 seconds. This is a cold runtime cache with reused Cargo artifacts, not a clean-build benchmark against earlier runs. Evidence: `/tmp/kuru-627-corrections-push.log`, `/tmp/kuru-627-corrections-cold-cache-path` and `target/coverage.lcov`.
- [x] 1.3 @runtime (agent) Run the actual GitHub-hosted Ubuntu, macOS and Windows native workflows on the changed task graph -> cold engine startup, native behavior, source installation and installed offline runtime pass, required aggregation stays enforced and observed preparation/total times are reported without claiming a controlled benchmark.

  Attempted at 1943c4e in CI `34551251564`. GitHub refused all 15 jobs before
  executing any steps, reporting failed account payments or a spending limit.
  There were zero artifacts and no new native or hosted static results. This
  external startup block does not satisfy native acceptance or invalidate the
  separately observed local cold-cache pass. Evidence:
  `/tmp/kuru-ci-1943c4e-results.md` and
  `/tmp/kuru-windows-1943c4e-results.md`. No retry or billing action was taken.

  Hosted execution resumed at the evidence-only commit 1234096 in
  [CI 34552780285](https://github.com/replygirl/kuru/actions/runs/34552780285).
  Ubuntu and macOS arm64 passed their instrumented suites, source installation
  and installed cold offline install/update checks. Their measured coverage was
  14,318/14,746 (97.097518%) and 14,329/14,757 (97.099682%), respectively.
  Input-only preparation took 6.79/7.98 seconds; their coverage task took
  920.65/294.76 seconds and the complete coverage graph took 927.47/302.77
  seconds. Ordinary memory prefetch did not run before instrumentation. These
  are observations from different native runners, not a controlled performance
  comparison. All static jobs and both other Unix shipping/memory jobs passed.
  Full Windows subsequently completed with 381 passing cases, eight failures
  and one ignored timing benchmark. Its coverage/preparation task took 2,537.35
  seconds. All 62 memory cases passed, including cold provisioning and lifecycle
  checks, but full-app update and other native consumer failures prevented
  combined LCOV and later source/shipping installation checks. The required
  aggregate correctly failed, so this acceptance row remains open. Evidence:
  `/tmp/kuru-ci-1234096-results.md` and
  `/tmp/kuru-windows-1234096-results.md`.

## 2. Workflow and instruction consistency

- [x] 2.1 @integration (agent) Run relevant format, tooling, strict cospec/managed and docs checks and review the resulting diff -> the concurrent relevant gate passed in 4.63 seconds, including docs build/content/links. Independent diff review found only the declared prerequisite and documentation edits; native jobs, thresholds, scoped tools and publication order are intact. Evidence: `/tmp/kuru-coverage-input-static.log` and `/tmp/kuru-coverage-input-graph-review.md`.

## Final integration: a1547ed

[CI 34589999794](https://github.com/replygirl/kuru/actions/runs/34589999794)
completed every required job; aggregate 103240993431 passed at 11:08:31Z on
September 11. Shared exact source and coverage details are in native-windows
verification, "Final native acceptance: a1547ed".

Each full native graph prepared only the host and Windows archive inputs before
one instrumented workspace suite. Ordinary prefetch/snapshot was not added to
coverage; ordinary tests retain their existing snapshot selection. All unchanged 90% gates,
actual cold-engine behavior, source installs and selected offline install/update
cases passed. Windows also passed all three offline Cargo controls and both final
shipping PE inventories. All static categories and the other Unix lanes passed.

| Full runner | Two input tasks | Instrumented task | Complete coverage graph |
| --- | --- | --- | --- |
| Windows | 20.00 / 20.06 s | 1370.41 s | 1390.59 s |
| Ubuntu | 3.80 / 4.20 s | 185.06 s | 189.29 s |
| macOS ARM | 10.86 / 11.69 s | 205.77 s | 217.50 s |

These are observed task durations under the actual runner/cache conditions,
not controlled comparisons or a fresh Cargo build benchmark. The separately
accepted 1943c4e absent-runtime-cache experiment remains the cold-cache proof.
Final local eight-hook acceptance also passed. Source and shipping timings remain separate
in the final reports, `/tmp/kuru-ci-a1547ed-results.md` and
`/tmp/kuru-windows-a1547ed-results.md`; raw task logs and
`/tmp/kuru-a1547ed-push.log` retain the measurements. No deadline, threshold,
dependency pin or Release/Pages scheduling changed in this prerequisite update.
