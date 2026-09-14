## Why

The Windows recovery fixture discarded its asynchronous memory-open result
while waiting for a DDL pause semaphore held by that same operation. An early
open failure was therefore reported only as a later child deadline, hiding the
actionable cause and preventing the shard from producing a receipt.

## What Changes

- Retain the fixture's opening task and select its result against the existing
  DDL pause boundary under the unchanged deadline.
- Report an early open error, task failure, and unexpected pre-pause success
  distinctly while leaving the reached-boundary process-loss behavior intact.
- Add a deterministic real-open regression proving an invalid startup setting
  surfaces its validation cause before the DDL boundary instead of timing out.

## Impact

Only `packages/kuru-memory/src/store/recovery_tests.rs` changes. Product memory
behavior, the 10-second child bound, the 25-second parent bound, process-loss
ownership, recovery assertions, and cleanup policy remain unchanged; final
native verification is still required in the ordinary Windows memory shard.
