## Context

The completed macOS ARM job in CI 34584264723 reported a 30-second timeout at
`Fixture::run` in `bootstrap_install.rs`, with no simulated host or partial
output. All four host cases use local release files. The two uname wrappers
only print the supplied OS and architecture; no host-mapping or transport defect
has been demonstrated.

Tokio's consuming `wait_with_output` concurrently waits for the child and output
EOF. Cancelling it loses buffered evidence and can leave the fixture's saved
group number after the root has been reaped. Its current Drop guard requests
termination without observing cleanup. Correct these fixture defects without
claiming they explain the historical installer stall.

## Decisions

Keep one fixture-local subprocess owner outside the timed capture future. Retain
the actual Child, stdout and stderr handles, bounded buffers, EOF flags and case
context. Use at most 4 MiB per captured stream and at most 64 KiB per diagnostic
excerpt; continue draining when retaining only a prefix. Preserve the ordinary
30-second bound. A shorter bound may be supplied only by the controlled timeout
regressions after their real startup handshake.

Observe root exit through the already-pinned safe rustix
`waitid(EXITED | NOHANG | NOWAIT)` API, with bounded yielding between observations.
Do not call Tokio wait/try_wait while the original group authority is needed.
On timeout or capture error, signal only the still-owned anchored group, then
await root reaping and actual pipe completion under a separate five-second
cleanup bound. Clear numeric signalling authority before reap. On the natural
completion path, require the root's observed exit and both output EOFs, disarm
numeric signalling, then reap and preserve the real status. Check group absence
with a read-only existence query after reaping. Only ESRCH establishes absence;
a present group, EPERM or another query error fails conservatively and retains
the private resources without signalling the now-unanchored number. This retains
the original rejection of surviving children instead of silently killing them
and reporting ordinary success. The read-only query can conservatively reject
an unrelated recycled number; it never grants signal authority.
An unexpected loss of wait ownership disarms saved numeric signalling and is an
error, not authority to act on a possibly reused ID. Never report uncertain or
failed cleanup as completed; include the original failure and cleanup result.

Label OS/architecture/target and the explicit argument context supplied by the
fixture. On failure, report root observation, elapsed time, stream byte counts,
EOF flags and partial output. Snapshot only known fixture installation-stage
names and metadata, with bounded enumeration; call these observations rather
than inferring a precise installer instruction. Do not mix Bash tracing with
stderr, which could make existing failure-substring assertions misleadingly
pass. No product failpoint or tracing option is introduced.

Use actual shell roots and descendants for two controls: a live root with a
partial response, and an exited root whose descendant retains an output pipe.
Synchronize on a private ready signal, then require distinct root observations,
retained sentinel output and observed pipe/root cleanup. Also exercise natural
nonzero exit while draining beyond the retained output bound, and an exited root
whose descendant has closed its output but retains a separately releasable
socket. The latter must fail the surviving-group check; release that real
descendant through the independent socket and observe EOF afterward. Add the regression
against the original capture behavior first; any minimal test-seam refactor must
preserve that original behavior for the recorded failing run. No compile error
or invented state tuple counts as RED. Keep existing host selection, installation,
malformed archive, limit, cancellation and unchanged-destination assertions.

## Integration contract

This is an owning Unix test helper using real Bash, local files, pipes and the
existing pinned rustix safe API. The dependency edge belongs in delivery's Unix
dev-dependencies so ordinary tests compile without a tooling-feature assumption.
No platform API, application subprocess facade or provider worker is introduced.
Unix root/group and actual controlled EOF evidence are not a Windows Job active
count or proof about arbitrary escaped descendants.

## Risks / Trade-offs

- Diagnostic code can hide the original failure: retain both original and
  cleanup causes, use finite buffers and preserve case-specific assertions.
- Root reaping can invalidate numeric authority: observe without reaping and
  disarm before authoritative wait, including unexpected wait-ownership errors.
- The historical stall may not recur locally: record controlled observation
  repair separately from unchanged-host successes; require actual full native
  integration before closing and retain any new failure honestly.
