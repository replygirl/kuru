# Design

## Context

Group signals can miss a child forked during the kernel's group walk. Reaping
the root immediately consumes the exact wait identity that reserves the group
number, so a post-reap rescue signal would be unsafe. The independent review
accepted retained-root cleanup and required one nonblocking shared step.

## Decisions

`pre_reap_step` operates only after the immediate initial transition. Running
roots are pending without snapshots. Exited roots retain one read-only worker
containing only the group number, cancellation flag and absolute deadline.
Completed worker cleanup precedes its result. Polls join only finished workers;
cancellation of an async caller retains the same job in its process owner.
The existing worker retains one Tokio completion receiver. It publishes the
wake only after snapshot helper cleanup and reader joins return. A bounded
platform wait wakes on completion or the caller's existing poll/deadline;
it never consumes listing results or signals. The tiny wake-to-thread-exit
gap uses cooperative yields within that same bound, never an unfinished join.
Cancelled waits retain the receiver and job. Synchronous shell cleanup uses
its existing owned runtime for this bounded wait, preserving post-reap backoff.
Helper observation interruption does not disarm; other observation errors
permanently disarm helper signals. Reader startup is fallible, with the owned
helper and any already-started reader retained through partial setup cleanup.

Only the retained process owner can repeat a group-then-root sweep. It obtains
fresh `waitid(EXITED | NOHANG | NOWAIT)` immediately beforehand. Interrupted
observation sends nothing; ownership errors permanently disarm. Descendant
zombies remain pending; live rows prompt a fresh sweep. Rows never grant signal
authority. Empty observations are consumed, allowing a resumed pre-reap owner
to inspect again rather than reuse an empty cache.

Snapshot worker admission uses an absolute deadline and a single spawn-lock
`try_lock`; busy admission is unobserved and the caller's loop retries. The
inspection deadline is the earlier of the caller deadline and worker admission
plus the existing snapshot bound. Expiry admits neither another worker nor a
repeated tree signal. Helpers own kill/reap and pipe drains through completion.
Membership helpers reject nonempty stderr even with exit zero, since native
macOS ps can report sysctl failure that way. Rich diagnostic callers retain
their existing behavior.

Pre-reap observation requests only pid, pgid and state. On macOS the documented
single `-g <group> -x` selector includes no-terminal members without UID
filtering; the helper explicitly sets COMMAND_MODE=unix2003 so inherited legacy
mode cannot change those arguments. Linux retains the all-user `-A` listing.
Rich diagnostic and post-reap permission listings remain unchanged.

Hooks, RPC, shell and coverage drive the step from their existing loops and
pass their existing absolute cleanup limit. Ready allows exact reap; expiry
allows the existing bounded fallback. Post-reap signal-zero/EPERM resolution
and output drains remain independent success gates. Drop stays immediate.
Hooks perform the existing exact-root reap fallback at expiry even if helper
cleanup is pending, then report cleanup unconfirmed. A late helper remains
under its read-only worker; it cannot make that fallback report success.
Coverage performs this fallback only at final process disposal, so a cancelled
wait can still resume its retained root and membership job. Disposal never
signals or converts an unconfirmed supervision result into success.
Retained-shell rounds poll pending pre-reap work, but return to once-per-round
read-only observations after the root is reaped.

## Risks / Trade-offs

One `ps` helper adds normal cleanup latency; measure fixed successful
hook/shell batches before and after, including concurrent admission. A listing
is not atomic containment evidence and may miss a concurrent fork; preserve
the final absence gate. Escaped groups remain outside scope.
Completion wakes remove avoidable caller polling sleep; native inspection
and the helper's existing polling cadence remain measured costs. Verify lost
wakes, cancelled/resumed waits, deadline/event ties, panic/disconnection and
publication before actual worker exit without premature joins or readiness.

Native spawn, scheduling, helper kill/reap and reader joins are not preemptible
hard real-time operations. Absolute admission/polling limits preserve responsive
caller deadlines; a helper already in native cleanup may finish under its owned
continuation. Late results never authorize tree signals.

## Operational surface

Native Unix process cleanup only, on existing macOS and Linux targets. No bind
address, secret, tool version, dependency or execution topology changes.

## Integration contract

The platform retains exact standard-child wait ownership and its private fresh
group. Stock `ps` supplies bounded read-only numeric membership, never authority.
Callers keep their primary errors, admission leases and pipe cleanup ownership.
