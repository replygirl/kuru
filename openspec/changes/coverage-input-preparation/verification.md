## 1. Cold instrumented execution [critical]

- [x] 1.1 @integration (agent) Resolve the actual pinned mise coverage and ordinary-test graphs -> twelve mise 2026.9.4 graph/info/dry-run queries passed. Coverage emits two delivery archive-preparation commands and one llvm-cov command, without an ordinary memory build/prefetch; its snapshot opt-in remains explicitly unset even when inherited as 1. Ordinary app/memory/runtime tests retain shared prefetch and snapshot=1. Windows-target dry-run was on macOS, not native execution. Evidence: `/tmp/kuru-coverage-input-graph-review.md` and `/tmp/kuru-coverage-input-graphs/`.
- [x] 1.2 @runtime (agent) Run the normal pre-push on macOS arm64 with a new private KURU_DOLT_CACHE -> all eight concurrent hooks passed at 1943c4e, with 14,328/14,757 instrumented lines (97.092905%). The cache was absent before the hook and afterward contained the actual Dolt executable and licenses. Coverage's resolved preparation executed only the two archive-input commands before its own instrumented suite; no ordinary memory prefetch or supervisor snapshot ran. Actual install/update, cold startup, durable SQL/reopen and runtime cases passed. Archive preparation took 1.19/1.57 seconds, the suite 260.25 seconds and coverage including preparation 261.90 seconds. This is a cold runtime cache with reused Cargo artifacts, not a clean-build benchmark against earlier runs. Evidence: `/tmp/kuru-627-corrections-push.log`, `/tmp/kuru-627-corrections-cold-cache-path` and `target/coverage.lcov`.
- [ ] 1.3 @runtime (agent) Run the actual GitHub-hosted Ubuntu, macOS and Windows native workflows on the changed task graph -> cold engine startup, native behavior, source installation and installed offline runtime pass, required aggregation stays enforced and observed preparation/total times are reported without claiming a controlled benchmark.

  Attempted at 1943c4e in CI `34551251564`. GitHub refused all 15 jobs before
  executing any steps, reporting failed account payments or a spending limit.
  There were zero artifacts and no new native or hosted static results. This
  external startup block does not satisfy native acceptance or invalidate the
  separately observed local cold-cache pass. Evidence:
  `/tmp/kuru-ci-1943c4e-results.md` and
  `/tmp/kuru-windows-1943c4e-results.md`. No retry or billing action was taken.

## 2. Workflow and instruction consistency

- [x] 2.1 @integration (agent) Run relevant format, tooling, strict cospec/managed and docs checks and review the resulting diff -> the concurrent relevant gate passed in 4.63 seconds, including docs build/content/links. Independent diff review found only the declared prerequisite and documentation edits; native jobs, thresholds, scoped tools and publication order are intact. Evidence: `/tmp/kuru-coverage-input-static.log` and `/tmp/kuru-coverage-input-graph-review.md`.
