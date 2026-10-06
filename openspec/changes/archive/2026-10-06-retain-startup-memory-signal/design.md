# Design

## Context

The independently archived import and Unix recovery implementations each correctly retained a signal during their own operation. Normal integration exposed a gap after pending recovery completed and before import registered a new receiver: Tokio's global handler was installed, but no retained receiver carried a signal across that interval.

## Decisions

Keep one optional invocation listener. Recovery inserts it only when a pending transaction needs it; the memory helper reuses it and polls queued cancellation before polling the operation. Once operation admission wins, interruption still awaits that same owned operation and returns its actual receipt/error. Update's own listener and Run's input-to-delivery listener remain unchanged. M1's future callers use the same third helper argument through normal integration.

## Operational surface

This is local CLI cancellation ordering on the supported binaries, with no endpoint, bind address, container topology, credentials, engine version, archive payload or installation workflow changes. Two private causal helper cases cover queued-before-admission refusal and accepted settlement. Existing real OS signal/import/recovery results remain scoped inherited evidence.

## Risks / Trade-offs

An accepted import can remain durable after interruption; preserve confirmed receipts and the existing honest error context. Do not reinterpret cancellation as rollback or change exit classification. Read-only commands and other commands' existing cleanup policies are outside this focused helper correction. Prior archives remain immutable.
