# Design

## Context

The publisher (`service/activity.rs` `publish`) writes the record on its own task, so nothing in the open, endpoint publication or serving waits for a write. After the open ends it writes any unwritten activity once and returns. Since #174, an open that counts always has such a write pending: the milestones after its last stage (`MainPool` … `StoreReady`) advance the count inside the 250 ms progress spacing, and the open's end makes it immediate. That write runs alongside endpoint publication, serving and the start of the close. The close joined the publisher only in `activity::retire`, inside `close_store_and_record`, which runs after `record.retire` has removed the endpoint. The retirement order (endpoint, then record, then store) was correct. What was missing was any point after which the record held still.

Readers: the client's `observe` treats only a raw OS not-found from the lookup as `Absent`; a handle whose name was replaced (Unix: unlinked, no OS code; Windows: delete-pending, `PermissionDenied`) or any other failure is `Unusable` and decides nothing. A Windows `ReplaceRegular` publication is one `NtSetInformationFile(FileRenameInformationEx, REPLACE_IF_EXISTS | POSIX_SEMANTICS)` call, so a lookup finds the old object or the new one and never a missing name. That last point is inferred from the documented semantics, not measured here. The test helper `read_activity` is stricter: any error, including a replacement between its open and its verify, fails the read. The flaked test collapsed that error to a boolean.

## Goals / Non-Goals

**Goals:** the record is settled before the endpoint retires, so a starter whose attach fails reads the record the open last wrote, or its failing mark, or its absence after retirement, and never a late replacement. Tests report the actual read error.

**Non-Goals:** changing the record format, the client's `observe`/`OwnerWatch`, the platform replace, any timeout or spacing, or making the open or serving wait for a write.

## Decisions

- **Await the last write in close, between the listener drop and the endpoint retire.** The close already waited for this publisher (in `retire`, or in `mark_failing`), so the wait only moves earlier in the same close. Rejected: joining before serving or at endpoint publication. Both are explicitly write-free, and a stalled write must never delay the open. Rejected: joining before the listener drop. Clients that connected meanwhile would wait in the backlog instead of being refused.
- **Join once and cache the last activity.** `Publisher::finish_writes` takes the task and keeps its returned activity. `mark_failing` and `retire` both call it. Otherwise the earlier join would leave `mark_failing` with no task, and an unserved owner would lose its failing reason without notice.
- **No reader change.** The product reader already separates "being replaced" (`Unusable`, retried at the next poll) from "retired" (`Absent`). The flake came from the test's strict reader, which met an unordered write.

## Risks / Trade-offs

- [A stalled last write keeps the endpoint published, with its listener dropped, for as long as the write takes] → Before this change the same stall delayed the record's retirement, the store close and the lock release by the same amount. A client meanwhile meets a refused connection, which it already handles as a closing owner. The write is bounded by its own I/O, and the publisher never rewrites after its open.
- [Could a real retirement look like a replace window?] → No. Retirement still renames the record off its name, and the next lookup reports a raw not-found, which is `Absent`. This change only removes writes that could land after the endpoint retired.
- [Could a starter's OwnerRetired or OwnerFailed verdict come later?] → The record is retired or marked at the same point in the close as before. The endpoint retire can move later by at most the write in flight, and the close's total length is unchanged.
