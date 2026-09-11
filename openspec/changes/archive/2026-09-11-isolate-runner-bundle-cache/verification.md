Current acceptance is complete at a1547ed after an actual Windows cache restore;
see final integration below and native-windows verification, "Final native
acceptance: a1547ed". Earlier failure/pending statements remain historical.

## 1. Private native build inputs [critical]

- [x] 1.1 @runtime (agent) Execute the reusable native job on GitHub's windows-2025 runner after a real Rust-cache restoration -> preparation uses the absolute runner-temporary bundle directory outside restored target outputs; both prerequisites, full instrumented tests and 90% gate, source install, three offline Cargo controls, selected shipping install/update and both native PE inventories pass without weakening private-object checks.
- [x] 1.2 @runtime (agent) Execute the same corrected job on its Ubuntu and macOS runners and complete the required CI graph -> preparation, native behavior/coverage and actual source/shipping checks pass from the selected job-local inputs, and the aggregate succeeds.
- [x] 1.3 @integration (agent) Run granular workflow/tooling/docs/cospec validation and normal hooks, and review the exact diff -> checks pass; package-owned preparation, Cargo artifact caching, versions, local defaults, ACL/integrity policy, required jobs and final Release/Pages topology are preserved.

## Observed failure before correction

CI 34588322751 full Windows job 103227553568 restored the 876,767,551-byte
Cargo cache from fallback key
`v0-rust-native-coverage-Windows_NT-x64-51494ef1-f9dcace3`. Its recorded paths
include `D:\a\kuru\kuru\target`. With no bundle-directory override, both actual
preparation tasks then failed opening the default private bundle directory with
`private object grants access to another principal` (completed log 447–458).
No full application test, full LCOV, source install, offline Cargo control,
shipping case or PE check ran. The precise rejected ACE and restoration's ACL
transformation were not logged; this evidence does not name an old account SID.
Evidence: `/tmp/kuru-windows-45a13b5-full.log`. The remaining graph was still
active when this record was authored. No retry or cache deletion was performed.

## Local validation and source review

Strict validation and actual apply both exited zero before implementation; all
four returned context files were read. The corrected workflow exports the
existing directory override through GITHUB_ENV before mise and Rust-cache. It
does not create the leaf, repair permissions, remove restored state or change a
cache key. Only the reusable native job and its owning guidance changed.

Actionlint, shell lint and repository metadata validation passed through the
granular tooling task in 1.06 seconds. The docs lint/format/build/link/content
graph passed in 2.88 seconds. Independent source review found no blocker and
traced the override through preparation, build verification, native ZIP fixtures,
source installation, offline controls and final prefetch. The offline script
restores its temporary override. Evidence:
`/tmp/kuru-runner-cache-tooling.log`, `/tmp/kuru-runner-cache-docs.log`,
`/tmp/kuru-windows-bundle-cache-45a13b5-investigation.md`.

The pinned Rust-cache cleanup can remove bundle files while retaining the
directory, so no restored Dolt archive bytes are claimed. The logs correlate the
prior saved cache's exact key and byte count with the current restoration, but
do not identify the rejected ACE. A subsequent native execution must demonstrate
actual restoration followed by successful private preparation outside target;
a cache miss or source review alone does not complete the runtime rows.

## Final integration: a1547ed

[CI 34589999794](https://github.com/replygirl/kuru/actions/runs/34589999794)
completed full Windows and required aggregate 103240993431 successfully on
September 11; the aggregate passed at 11:08:31Z. Shared exact source, metrics
and graph details are in native-windows verification, "Final native acceptance:
a1547ed".

The actual Windows cache restore used fallback key
`v0-rust-native-coverage-Windows_NT-x64-51494ef1-f9dcace3`, 876,767,551 bytes,
fullmatch=false (completed log 406–416). Both subsequent preparation commands
passed and printed the same verified archive under `D:\a\_temp\kuru-bundles`
(455–457), outside the restored target directory. This is the same restored key
and byte count as the earlier failed run, not a cache miss. The early export did
not create the leaf, repair an ACL, delete a cache or change its key; private
creation/validation remained with the existing preparer.

Source-build and shipping preparation returned the same mirror (1415, 1549).
The three offline Cargo controls retained/restored their selected override and
passed, as did full Windows behavior/coverage, the final real prefetch and both
Kuru/Dolt PE inventories. Actual selected source-installed Kuru completed cold offline
direct installation and self-update, retaining the unchanged integrity policies.

Ubuntu and macOS ARM independently used `/home/runner/work/_temp/kuru-bundles`
and `/Users/runner/work/_temp/kuru-bundles` through initial preparation, source
installation and shipping. Their full suites passed above 97% coverage; all four
Unix lanes, seven static categories, lock checks and normal eight local hooks
passed. Evidence:
`/tmp/kuru-windows-a1547ed-results.md`, its `-cache-bundle-proof.txt` and completed
logs, `/tmp/kuru-ci-a1547ed-results.md`, and `/tmp/kuru-a1547ed-push.log`.

The earlier rejected ACE and any restored Dolt archive bytes remain unknown.
This proof concerns current runner-owned preparation outside restored outputs;
it changes no local default, package task ownership, deadline, ACL/hash policy,
dependency pin or Release/Pages topology. The public release and live provider
authentication boundaries remain separate and unperformed.
