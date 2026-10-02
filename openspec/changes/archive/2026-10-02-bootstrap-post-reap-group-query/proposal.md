# Proposal

## Why

Main went red on macOS (run 37001606321, job 110820550523, commit bc501f2e):
`checksum_missing_corrupt_ambiguous_and_malformed_inputs_preserve_the_installation`
failed with `post-reap process group query: Operation not permitted (os error 1)`
40 ms into a capture whose bootstrap had exited and been reaped normally. The
test-support cleanup (`packages/kuru-delivery/tests/support/bootstrap_process.rs`)
asks signal zero about the reaped root's numeric process group and treats every
answer other than `ESRCH` as a failure. Once the root is reaped the number no
longer identifies anything of ours: macOS recycled it to a process of another
user, signal zero was refused with `EPERM`, and a cleanup that left nothing
behind was reported as unconfirmed.

The same shape exists wherever a post-reap numeric query decides cleanup.
`OwnedProcessGroup::presence_after_reap` callers in hooks, RPC, the Unix shell,
the coverage runner and the bounded delivery command poll signal zero until
their deadline and treat persistent `EPERM` as failure; delivery, platform and
runtime tests assert `ESRCH` (or a `kill -0` failure) on numeric IDs after reap.
A recycled group fails them all spuriously, and a numeric pid reused by one of
our own concurrent test processes fails the single-pid checks the other way.

## What Changes

- `OwnedProcessGroup::presence_after_reap` stays a single signal-zero query.
- New `kuru_platform::unix::PermissionListing`, created per cleanup with that
  cleanup's deadline, resolves `GroupPresence::PermissionDenied` with at most
  one bounded `ps` listing of the group's members, run off the async executor
  (`spawn_blocking`, or directly for the already-blocking shell cleanup) and
  limited to `min(SNAPSHOT_TIMEOUT, deadline - now)`. The listing classifies:
  - no listed member: `Absent`;
  - only members whose real and effective user IDs differ from ours:
    `Recycled` (new `GroupPresence` variant), cleanup succeeds and the listing
    is retained as diagnostic evidence;
  - a member of ours (zombies included) or an unavailable listing:
    `PermissionDenied`, so the caller keeps its cheap signal-zero poll and
    fails at its unchanged deadline.
  No listing happens on `ESRCH`, on success, after the first listing, or with
  no budget left, so no cleanup bound grows and no retry or sleep is added.
  The Unix shell's retained-ownership rounds (which already retry with a
  100 ms to 1 s backoff after a bounded cleanup failed) get one listing per
  round, bounded by that round's backoff interval, on their dedicated thread.
- `group_presence_after_reap(group)` is the numeric equivalent of
  `presence_after_reap` for the bounded delivery command, which owns only its
  reaped tokio child: one signal-zero query, rejecting group numbers 0 and 1.
- Read-only `observe_group_after_reap` / `GroupObservation` classify a reaped
  numeric group for test assertions with the same rules (a live group lists its
  members for the diagnostic). `snapshot` gains `uid`/`ruid` columns,
  `group_members(_within)`, `processes` and `still_listed`.
- Test sites stop treating numeric IDs as identity: the bootstrap capture
  cleanup, delivery `advisory` and coverage tests, the hook group helper, the
  bootstrap producer checks (recorded `(pid, command)` rows or a per-fixture
  `exec -a` command tag) and the runtime owned-shell shutdown check. The
  owned shell is the test process's direct child, so after `ESRCH` is not
  returned it remains exactly while a fresh listing shows its ID under this
  parent, running or as an unreaped zombie; a recorded `(pid, command)` row
  would miss the zombie, whose arguments `ps` no longer prints.
- Nothing here authorizes a signal: every new path is signal zero or `ps`.

### Decision: `EPERM` followed by an empty listing is `Absent`

The listing runs after the refusal and lists every process of ours (only other
users' processes can be hidden, for example by Linux `hidepid`). An empty
listing is direct evidence that no process of ours was in the group at that
instant, and a group cannot gain a member of ours again except through a new
process that the caller did not create. The decision therefore rests on the
listing, not on why signal zero was refused.

Measured on this macOS 27.0 arm64 host (Perl `kill`/`waitpid` probes, run
from the session scratchpad, not committed):

- A group whose only member is an exited, not yet reaped process: signal zero
  and `SIGKILL` were both refused with `EPERM` in 200 of 200 trials. macOS
  refuses signals to a zombie-only group; this is the case the comment at
  `packages/kuru-delivery/src/command.rs:313` describes for the pre-reap kill.
- 4000 spawn/exit/reap/signal-zero cycles (2000 bare leaders, 2000 leaders
  with a short-lived background child): `ESRCH` 2002 times, success 1998
  times, `EPERM` never.

Inferred from those two measurements, not observed directly: after the reap,
`EPERM` with no listed member arises when the group's last members of ours were
zombies at the signal (for example a reparented background child awaiting its
new parent's reap) and were reaped before the listing. A listed zombie of ours
keeps `PermissionDenied`. The CI failure itself (`EPERM` 40 ms after a natural
exit) matches a group number recycled by another user, which is the case the
regression tests reproduce with a real foreign-uid group.

### Accepted residual

If the single listing shows a member of ours and that member then exits and
the number is recycled by another user before the next 10 ms poll, the cleanup
reports `PermissionDenied` until its deadline and fails. That is a false
failure, never a false pass, and it keeps the listing to one per cleanup.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The native-platform and provider-tools requirements already demand
observed group absence; only the implementation's observation of it was wrong.

## Impact

- `packages/kuru-platform/src/unix.rs`, `src/unix/snapshot.rs` and tests
  `unix_snapshot.rs`, `unix_process_group.rs`: new `PermissionListing`,
  `GroupPresence::Recycled`, `GroupObservation`, `snapshot` columns and helpers.
- `packages/kuru-connectors/src/{hooks,rpc,unix_shell}.rs`,
  `packages/kuru-delivery/src/{command,coverage}.rs`: thread one
  `PermissionListing` through each cleanup loop.
- `packages/kuru-delivery/tests/{bootstrap_install,advisory}.rs`,
  `tests/support/bootstrap_process.rs`, `packages/kuru-runtime/src/review_tests.rs`:
  post-reap assertions use listed evidence instead of numeric identity.
- No dependency, configuration, documentation or workflow change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
