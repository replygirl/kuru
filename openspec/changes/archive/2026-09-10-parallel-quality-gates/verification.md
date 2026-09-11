## 1. Concurrent checks and complete gates [critical]

- [x] 1.1 @integration (agent) exercise independent local mise/hk checks, including a controlled failed task -> actual mise 2026.9.4 and hk 1.58.1 FIFO rendezvous requires both workers to overlap; success exits 0 and controlled failure exits 23 with both workers completed. Actual repository hk plan places all eight pre-push checks in one group. Resolved mise graph has one npm ci, one workspace llvm-cov and no ordinary cargo test; coverage preserves its instrumented supervisor. Evidence: `/tmp/kuru-parallel-task-evidence.md` and `/tmp/kuru-parallel-proof/results.log`.
- [x] 1.2 @integration (agent) validate workflows and required-result aggregation with success, failure, cancellation and skipped inputs -> actionlint passes; executing both workflows' actual jq predicates accepts only all-success, rejecting failure, cancellation, skipped and empty results. Review confirms every branch is required, exact release source refs, all five builds, and final post-publication Pages. Validation artifacts cannot match the release asset glob and have attempt-specific names. Evidence: `/tmp/kuru-parallel-gates-review.log`.
- [x] 1.3 @runtime (agent) run PR CI on GitHub-hosted Ubuntu, macOS and Windows -> static categories appear once as independent jobs; native coverage executes one instrumented suite per leg; native installation/update acceptance and required aggregate remain present; observed outcomes and incomplete application acceptance are recorded below.

## 2. Tooling and workflow discipline

- [x] 2.1 @integration (agent) execute granular local format/lint/typecheck/tooling/cospec/docs/coverage checks -> all eight concurrent pre-push steps passed at eb6a04b, including 14,307/14,727 covered lines (97.1481%) and the exact docs toolchain. Evidence: `/tmp/kuru-npm-server-fixes-push.log`.
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

## Completed scheduling acceptance

[CI 34544299862](https://github.com/replygirl/kuru/actions/runs/34544299862)
at `eb6a04b28d2f40adb90a68f0b299aa2bfa06baa4` passed all seven static jobs and
their aggregate. They started independently at 23:56:23–25 UTC on September 10;
the aggregate finished at 23:57:36. The documentation job explicitly observed
Node 26.8.2 and npm 12.0.2, installed the locked dependencies and passed format,
lint, build and content checks without backend/lock mismatch warnings. Evidence:
`/tmp/kuru-windows-eb6a04b-docs.log` and the run's completed job records.

The preceding d66780f run passed one instrumented Windows primitive suite and
one macOS ARM workspace suite followed by source installation and the installed
offline runtime. Both additional native architecture jobs passed their memory
and packaged offline checks. Ubuntu passed its single instrumented suite, then
the new push automatically superseded it during source compilation; installation,
shipping acceptance and coverage upload did not complete on that job. Full
Windows compilation failed on the separately corrected native server fixture.
The required aggregate failed, as expected. No manual cancellation, release or
Pages dispatch was used.

These observations complete the CI scheduling change. They do not complete
`native-windows` or its required database, ConPTY, bootstrap, update and native
mise acceptance. Those application checks remain required in the workflow and
are still running on eb6a04b. The corrected local pre-push took 248.25 seconds
for coverage and preparation, with every static check also passing concurrently;
the unchanged workspace coverage threshold remains 90%.
