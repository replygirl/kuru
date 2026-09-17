## Context

A cold Windows provision executes a private copy of `dolt.exe` from inside its own `.install-*` stage, publishes the verified candidate, then closes the stage under the retained cache lease. `StagedActivation::finish_published` already drops the candidate source directory and the checked cold probe before that close, so Kuru holds no handle into the stage when cleanup starts. Windows can still refuse the first checked delete of the just-executed probe copy with `ERROR_ACCESS_DENIED` while its image section is torn down. Because nothing had been removed yet, `Directory::remove_tree` correctly reports `PublicationPhase::Rejected`, and `recoverable_child_removal` accepted rejected results only for raw OS32, so the bounded window never opened.

Native CI proves the holder is transient rather than a Kuru handle: the same `kuru-memory` tree failed `real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution` on one branch head (run 35219037077) while the same test passed on another head containing the identical provisioning code (run 35219035614), where a sibling fixture failed instead with the same access-denied cause at a different call site.

## Goals / Non-Goals

**Goals:** Let a rejected native access-denied child removal share the existing two-second reconciliation window; keep every identity, no-mutation and first-cause guarantee; keep post-publication cleanup failure a reported error.

**Non-Goals:** No platform API change, no widened or configurable deadline, no swallowed cleanup error, no publication retry, no process supervision, no change to `PublicationPhase` semantics.

## Decisions

`recoverable_child_removal` accepts `Rejected` with native raw error 5 in addition to its current `Rejected` raw 32 and every `Uncertain` result. A rejected removal is by definition one where no object changed, so retrying it after re-opening the exact retained child identity is safe; the loop still re-checks the outer and child identities before each attempt and refuses a replacement. This restores consistency with the same module's `pending_open_error`, which already treats native error 5 on an outer or child open as a pending name to reconcile, and with `activate_staged_with`, which already retries a proven-no-move activation only for native error 5. Retry policy belongs here rather than in `kuru-platform`, whose `remove_tree` states that "the caller owns inventory and retry policy".

Exhaustion is unchanged: `wait_for_cleanup_retry` still returns the first typed cause under the bounded-recovery context, so a genuine permission denial fails with its original text after at most two seconds instead of silently passing.

## Risks / Trade-offs

A private stage that is denied for a real access-control reason now occupies the full two-second budget before reporting the same first cause. That cost is bounded, and no cleanup outcome becomes non-fatal. The cache lease stays owned throughout, so the window cannot expose a partially cleaned stage to another installer.

## Operational surface

Windows-only private-stage cleanup inside the managed engine cache and its native fixtures. No bind address, secret, bundled asset, binary version, arch or CI topology changes.
