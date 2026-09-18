## Context

`Directory::remove_tree` enumerates a checked private tree and, for each enumerated name, opens fresh by-path handles (pinned, then movable) before verifying identity and deleting. Between the enumeration snapshot and those opens the name can legitimately disappear: on Windows a delete-pending entry is still enumerated and vanishes when its last handle closes — the case a cold provision hits with the private `dolt.exe` copy the checked probe just executed — and on unix an entry removed by a concurrent or earlier attempt behaves the same way. Every open failure in that loop is mapped through `phase(removed)`, which is `Rejected` while nothing has been removed yet, and `RemovalError` always reports the tree root as its path, so the CI log cannot distinguish a missing child from a missing root.

`kuru-memory`'s bounded cleanup treats `Rejected` as recoverable only for native raw 5 and 32, so raw 2 took the fatal arm on the first attempt and the published-but-uncleaned stage failed the run. Adding raw 2 there would paper over a mis-mapping at the wrong layer.

## Goals / Non-Goals

**Goals:** make "the target is already absent" the completed outcome of a checked removal at the layer that observes it, on both backends, with the final absence check still proving the result; keep a genuine rejection rejected.

**Non-Goals:** no retry, window, timeout or `recoverable_child_removal` change; no widened `PublicationPhase` semantics; no change to identity, privacy, retention, symlink/junction or depth checks; no memory-layer policy change.

## Decisions

Absence is mapped to completion at three Windows frames and two unix frames, guarded strictly by `io::ErrorKind::NotFound` / `Errno::NOENT`:

1. `windows::remove_children` — a per-entry pin, movable open, or the `info` behind either returning `NotFound` continues to the next enumerated entry. `removed` stays untouched: this attempt performed no removal, so a later failure keeps its `Rejected` phase and never becomes a false `Uncertain`.
2. `windows::remove_tree` — a root `pin_directory` returning `NotFound` skips child removal and the final unlink and falls through to the existing `symlink_metadata` absence check, which is what proves the outcome. Ancestor pins are unchanged: a vanished ancestor stays a rejection.
3. `windows::remove_empty_directory` — a `NotFound` at the DELETE open means the directory name is already gone; that is the operation's outcome. It applies equally to a nested directory and to the root.
4. `unix::remove_children` — `Errno::NOENT` on the child `openat`, in both the directory and the `NOTDIR` re-open arms, continues to the next entry.
5. `unix::remove_tree` — a root `verify_named` reporting `NOENT` runs the existing `verify_absent` and parent `sync_all` and returns success.

Everything else in those functions is untouched, including the identity comparison between the pinned and movable handles, which still rejects a name rebound to a different object.

The public `Directory::remove_tree` still `revalidate()`s the held root before entering the native backend, so a root that is already absent when the caller starts is rejected there exactly as before; frames 2 and 5 cover only the race that opens after that revalidation. That is deliberate: a caller asking to remove a handle it no longer holds is a different error from a removal completing.

Deterministic coverage needs a seam, following the repository's existing precedent of a test-only observer around a real filesystem operation (`files::move_directory_observed`). `fs::enumeration_seam` is a `#[cfg(test)]`, thread-local, crate-private observer invoked by both backends on each enumerated entry after its name is observed and before any handle on it is opened — the exact window the defect lives in. It is compiled out of every non-test build and is not reachable from `kuru-memory`, which is why the memory-layer regression fixture in the original plan is not added: the memory layer's policy is unchanged, and the mis-mapping is asserted at the layer that produces it.

## Risks / Trade-offs

A name that disappears for an unrelated reason during a removal now completes that removal instead of failing it. This is the operation's own contract — its success condition is disappearance, which the final absence check still verifies — and it cannot delete a new occupant, because a name is only skipped, never acted on. The narrow `NotFound` guard keeps denial and sharing violations, which are the real "someone still holds this" signals, unchanged.

## Operational surface

Checked filesystem tree removal on both supported platforms, exercised by managed engine-cache stage cleanup. No bind address, secret, bundled asset, engine version, arch or CI topology change.
