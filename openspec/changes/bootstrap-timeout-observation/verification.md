## 1. Observable owned timeout [critical]

- [x] 1.1 @regression (agent) Run the real controlled live-root and exited-root/retained-writer scenarios against the original capture behavior, then the corrected fixture -> original execution fails the required labeled partial-output/root-state assertions; corrected execution distinguishes both actual states, retains output and observes cleanup within its bound. Compiler failures and a plain non-reproducing host rerun are not RED/GREEN evidence.
- [x] 1.2 @runtime (agent) Observe the controlled startup handshake, bounded captures and cleanup of actual roots/descendants -> the ordinary 30-second timeout remains unchanged, saved numeric authority is disarmed before reap, stdout/stderr EOF and root status are recorded, and no controlled process or held pipe survives reported completion.
- [x] 1.3 @integration (agent) Execute all existing bootstrap cases with the real system Bash and local archive fixtures -> host mapping, checksums, size limits, archive rejection, cancellation and unchanged old bytes remain correct; diagnostics name the actual case and preserve bounded known-stage metadata without contaminating product stderr assertions.

## 2. Repository integration [critical]

- [ ] 2.1 @integration (agent) Run granular format/lint/typecheck/tooling/docs/cospec checks and the normal concurrent hooks -> checks pass with the existing pins, one relevant instrumented suite and at least 90 percent meaningful line coverage.
- [ ] 2.2 @integration (agent) Complete required native CI on the corrected candidate -> full Unix and Windows behavior/coverage, actual shipping/source checks and aggregate pass; any recurrence retains its concrete observation and no historical stall cause is inferred from a later pass.
- [ ] 2.3 @manual (agent) Review the final fixture and owning records -> no installer, deadline, release/Pages topology, user credential or provider behavior changed; strictly validate and actually archive after all required evidence completes.

## Initial evidence

CI 34584264723 at e7eb136 completed macOS ARM job 103214706134 with 375 passing
tests, one failed and three intentional exclusions. The sole failure was
`host_detection_selects_each_supported_archive_and_rejects_unknown_hosts` at
`bootstrap_install.rs:147`; its binary passed 12 other cases. Full macOS LCOV,
source installation and shipping checks were skipped. The remaining run was
still active when this record was authored. Investigation:
`/tmp/kuru-bootstrap-e7-investigation.md`; completed log:
`/tmp/kuru-ci-e7eb136-macos-arm.log`. No causal repair or native completion is
claimed by this planning record.

## Actual failing regression

The owning delivery mise test compiled the unchanged capture behavior with its
two new real controls in 3.77 seconds. Both controls completed their startup and
non-reaping state preconditions, then observed EOF on an independent inherited
socket after the expected timeout panic. Their required case/root/EOF/sentinel
and observed-cleanup assertions failed because the original panic retained only
`Elapsed(())`: zero passed, two failed in 0.36 seconds, task exit 101.
The ordinary 30-second bound is unchanged; the controls use 250 milliseconds
only after startup. This is actual macOS behavioral RED, not a compiler failure
or a reproduction of the historical installer stall. Exact pre-repair source:
`/tmp/kuru-bootstrap-observation-red.rs`; completed run:
`/tmp/kuru-bootstrap-observation-red.log`. The existing-pinned Unix rustix test
edge required no Cargo.lock change. The repaired controls and full acceptance
are recorded below.

## First repaired execution and natural-exit correction

The two controlled timeout scenarios passed on macOS in 0.36 seconds after a
1.51-second compile, owning task exit zero. Their independent socket EOF,
partial-output, actual distinct root observations and awaited-cleanup assertions
all executed. This is the controlled GREEN corresponding to the recorded RED,
not acceptance of the whole helper: `/tmp/kuru-bootstrap-observation-green.log`.

The subsequent owning delivery task failed, exit 101. Its bootstrap binary passed
the two timeout controls and failed the other fourteen cases. Each natural-exit
case reached root reaping and both output EOFs, but the new helper's unconditional
group termination returned macOS EPERM and correctly prevented a cleanup-success
claim. The new 4 MiB-plus-one drain control also observed the full byte count and
natural exit before that cleanup error. Evidence:
`/tmp/kuru-bootstrap-observation-suite.log`. Correct the natural-exit policy and
preserve detection of surviving children before accepting the helper; do not
blanket-ignore EPERM or infer the historical hosted timeout's cause from this
separate local implementation failure.

## Corrected default-feature execution and review

Package-context `mise exec -- cargo test -p kuru-delivery --locked --test
bootstrap_install` compiled in 4.35 seconds and passed all 17 cases in 5.19 seconds,
exit zero, with no tooling or all-features flag. This includes the original
thirteen installation/rejection/cancellation cases, both timeout observations,
the 4 MiB-plus-one drain with natural exit 17, and the output-closed live child
rejection. The latter independently proved its socket stayed open after the
helper rejected the leak, then released that exact child channel and observed
EOF before private cleanup. Evidence:
`/tmp/kuru-bootstrap-observation-default.log`.

Strict owning delivery Clippy passed in 0.78 seconds; the concurrent granular
code-format graph passed. Cargo.lock remained unchanged. The final independent
source review was CLEAR, confirming both natural and exceptional ownership paths,
retained uncertainty, output bounds, unchanged deadlines and original status
assertions: `/tmp/kuru-bootstrap-observation-green-review.md`. Revised strict
cospec validation also passed. Full hooks and the next required native graph
remain pending; neither this local pass nor the fixture repair establishes the
cause of the historical hosted stall.
