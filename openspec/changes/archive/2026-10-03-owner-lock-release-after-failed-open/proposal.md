# Proposal

## Why

`service::activity::tests::a_failed_endpoint_publication_marks_then_retires_the_record`
failed on main 23c9ce14 (run 37115918555, job 111182769749, macos-latest
coverage partition 2) at its last assertion: after `ServiceOwner::open` had
returned its expected error, marked and retired its record and closed its store,
a one-shot try of the project's owner lock found it still held. `ServiceLock`
had no `Drop`; only `ServiceLock::release` unlocked explicitly, and the failed
open's three error returns dropped the lock, closing its descriptor alone. The
owner lock is a `flock` on an open file description, and a child that any other
thread of the process is spawning holds a duplicate of every description between
fork and exec (the lock file is `O_CLOEXEC`, so only until exec). Closing one
descriptor does not end the lock while that duplicate lives, so the lock outlived
the failed open. In the product a starter elects a successor on this lock with a
non-waiting try, so the same race reports an owner that does not exist until the
starter's next poll.

This is the mechanism fixed for the template key lock in
`template-key-lock-upgrade-busy`; the service lock's explicit release already
existed, but its drop did not use it.

## What Changes

- `ServiceLock` unlocks explicitly before its handle closes on every ending:
  `release()` as before, and now its drop, so every error return, a dropped
  `MaintenancePermit` and every probe release the lock at once. A dropped
  `MaintenancePermit` releases its owner lock before its start lock, the
  reverse of acquisition, by field order.
- A failed `ServiceOwner::open` whose store had opened releases its owner lock
  explicitly after `close_store_and_record` (record marked, store closed and
  Dolt reaped, record retired) and before it returns; a release failure is added
  to the returned error's context.
- No wait, retry, bound or gate is added; the one-shot owner election and the
  reap-before-release order are unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

None. `project-memory-owner` already requires a failed open to retire its record
"before the owner releases its owner lock" and every ending to release only
after Dolt cleanup; the implementation let a transient duplicate of the
released description keep the lock held past the open's return.

## Impact

- `packages/kuru-memory/src/service.rs`: `ServiceLock` (`Drop`, `file()`,
  `release`), `MaintenancePermit` field order, `ServiceOwner::open_hooked`
  failure path, test hook use.
- `packages/kuru-memory/src/service/activity.rs`: `OwnerHooks::duplicate_owner_lock`
  (test-only) and the regression test.
- Docs: `docs/development.md`.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
