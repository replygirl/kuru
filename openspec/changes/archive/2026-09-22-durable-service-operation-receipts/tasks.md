## 1. Versioned branch receipts

- [x] 1.1 Add a registered main migration for compact retained operation receipts, preserving legacy rows and exact existing history; verify fresh, v1/v2/v3 upgrade, failed-publication and old-binary refusal fixtures.
- [x] 1.2 Stage and validate the permanent usage branch's receipt-shape upgrade before ledger writes, leaving historical candidates unchanged; verify branch heads/rows/schema and lost-upgrade-reply recovery with actual Dolt.
- [x] 1.3 Replace sole-receipt cleanup only for upgraded writable views and update version-aware validators/operational-GC fixtures; verify indexed lookup after many later writes and historical v1–v3 behavior.

## 2. Logical mutation identity and outcome

- [x] 2.1 Carry one caller-retained logical mutation UUID through typed IPC and existing store receipt insertion for unit writes, key it by store/view, hash method/view/canonical arguments without storing a payload copy, and reject conflicting work on the same scoped key; verify exact retry, mismatch and cross-view non-alias fixtures.
- [x] 2.2 Add a bounded typed outcome query with in-flight, committed, definitively absent and still-uncertain results, checking original handler completion or verified owner/Dolt reap before absence; verify cancellation and crash boundaries with actual Dolt.
- [x] 2.3 Bind single-attempt candidate creation to a caller-retained logical UUID and exact owned ref, preserve that ref on attachment loss, and use read-only repeatable outcome/reattach queries without replaying Begin or recreating missing/resolved refs. After a lost candidate unit-write reply, prove its indexed receipt and reattach the original still-open candidate before clearing the fence. Resolve usage ledger from existing invocation natural keys and candidate transitions from exact refs/base-target revisions, retaining conflicts and never returning an obsolete-generation handle; verify settled, absent and conflict cases.

## 3. Client containment and integration

- [x] 3.1 Fence every clone, candidate and ledger mutation on an incomplete mutating reply, retain its original logical ID, and permit further mutation only after typed resolution; verify definite rejection, cancellation and sibling-attachment behavior.
- [x] 3.2 Exercise a lost reply followed by a sibling write and owner restart through the native service with one real Dolt engine; verify both effects occur at most once and indexed outcome query remains exact without replay.
- [x] 3.3 Update receipt/schema memory documentation and the dependent `project-memory-service` blocker without claiming ordinary managed CLI attachment; run package format/lint/typecheck/native tests and strict Cospec validation before archive. Record combined coverage and exact-head hosted Windows/macOS/Linux service checks as post-commit delivery gates, with their results tracked separately from local archive evidence.
