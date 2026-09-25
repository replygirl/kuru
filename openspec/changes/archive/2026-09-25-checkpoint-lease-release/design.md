## Context

`CheckpointStore::lease` opens a checked `checkpoint.lock` object and calls
`File::try_lock`; on macOS this is a `flock` over the open file description.
The lease currently owns only a `File` and has no explicit release on Drop.
The CI `WouldBlock` is observed, while any particular concurrent child spawn
that may have prolonged its descriptor remains an inference.

## Goals / Non-Goals

**Goals:** Make the end of the owned lease release the actual advisory lock,
including when another descriptor refers to the same open file description.

**Non-Goals:** Add waits or retries, change lock-file identity, loosen active
owner exclusion, or infer that the historical CI failure had one proven cause.

## Decisions

1. **Unlock the owned file at lease Drop.** The lease controls the interval of
   exclusive mutation and validation. An explicit `File::unlock` releases that
   open-description lock when the interval ends; merely closing one descriptor
   does not if a duplicate survives. A retry around the next `try_lock` is
   rejected because it would hide the ownership error and alter refusal timing.
2. **Exercise a duplicated handle without a spawned child.** A test-local
   `try_clone` of the real held file creates the same open-description lifetime
   that a pre-exec duplicate can create, without a timing-sensitive process
   fixture. It checks both active-owner exclusion and post-Drop acquisition.

## Risks / Trade-offs

- [The Drop path cannot return an unlock error] → Scope the action to the held checked lock and prove immediate reacquisition in the regression; do not claim the historical CI spawn timing is established.
- [Unlock could release mutual exclusion early if a lease remains in use] → Only the lease's own Drop unlocks; a separate active lease still excludes fresh acquisition in the test.
