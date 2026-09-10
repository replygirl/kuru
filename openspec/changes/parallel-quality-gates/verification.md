## 1. Concurrent checks and complete gates [critical]

- [x] 1.1 @integration (agent) exercise independent local mise/hk checks, including a controlled failed task -> actual mise 2026.9.4 and hk 1.58.1 FIFO rendezvous requires both workers to overlap; success exits 0 and controlled failure exits 23 with both workers completed. Actual repository hk plan places all eight pre-push checks in one group. Resolved mise graph has one npm ci, one workspace llvm-cov and no ordinary cargo test; coverage preserves its instrumented supervisor. Evidence: `/tmp/kuru-parallel-task-evidence.md` and `/tmp/kuru-parallel-proof/results.log`.
- [x] 1.2 @integration (agent) validate workflows and required-result aggregation with success, failure, cancellation and skipped inputs -> actionlint passes; executing both workflows' actual jq predicates accepts only all-success, rejecting failure, cancellation, skipped and empty results. Review confirms every branch is required, exact release source refs, all five builds, and final post-publication Pages. Validation artifacts cannot match the release asset glob and have attempt-specific names. Evidence: `/tmp/kuru-parallel-gates-review.log`.
- [ ] 1.3 @runtime (agent) run PR CI on GitHub-hosted Ubuntu, macOS and Windows -> static categories appear once as independent jobs; native coverage executes one instrumented suite per leg; native installation/update acceptance and required aggregate remain present; record actual outcomes without relabeling unrelated application failures as passes.

## 2. Tooling and workflow discipline

- [ ] 2.1 @integration (agent) execute granular local format/lint/typecheck/tooling/cospec/docs/coverage checks -> existing meaningful coverage threshold and managed-file/repository invariants pass.
- [x] 2.2 @integration (agent) review release validation refs and job dependencies -> source-quality/source-tests use github.sha; verify/verify-tests use needs.bump.outputs.sha; plan/build require their respective pairs. Publication waits for all five builds and notes; build-docs/deploy-docs stay last. Independent review agreed with the executable graph inspection. No release or Pages workflow was dispatched.

## Observed integration and required correction

The real concurrent pre-push hook passed all eight checks for d66780f, with
14,307/14,727 covered Rust lines (97.1481%). Static steps finished within 6.4
seconds; coverage including preparation finished in 259.54 seconds, compared
with the prior 440.47-second pre-push on this host. Both used warm caches and
different source revisions; this is observed elapsed time, not a controlled
performance benchmark. Evidence: `/tmp/kuru-parallel-quality-push.log`.

CI run `34542165125` launched all seven static categories within two seconds
and passed them and their quality gate. Native Windows primitives ran one
instrumented suite (54 cases), and macOS ARM ran one instrumented workspace
suite followed by installed shipping-binary acceptance. Windows application
compilation exposed a separately tracked native fixture error. A green docs
job also exposed incorrect npm activation: root task discovery selected a
different backend from the app lock and used bundled npm 11.19.1.

The correction registers only the backend alias at root, retaining app-owned
version pins, installation and verification. An always-run helper compares
actual executable versions against app-scoped `mise current`, preserves child
failure statuses and bounds probes to 30 seconds. Required runtime acceptance
remains open until CI proves npm 12.0.2 activation with this correction. Fresh
private tool installation, cached stale-version rejection, failed child probes,
and the corrected local documentation graph passed; logs are under
`/tmp/kuru-npm-monorepo-proof`.
