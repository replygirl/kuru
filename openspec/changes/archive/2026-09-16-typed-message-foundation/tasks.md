## 1. Canonical content types

- [x] 1.1 Implement ordered message/completion blocks, metadata and derived accessors; verify mixed round trips, legacy exact text, unknown tag errors and usage absence versus zero in core tests.
- [x] 1.2 Adapt runtime input, bounded receipt/history, completion persistence and dream/journal callers; verify current receipt ID/redaction bounds and final TurnOutput/exact-retry behavior in runtime fixtures.

## 2. Durable migration and export

- [x] 2.1 Implement production v3 format discrimination and branch-version readers/writers; move test-only future migration and verify actual old-schema upgrade/reopen, candidate/history compatibility and malformed payload refusal.
- [x] 2.2 Preserve typed checkpoint receipt reconciliation and literal legacy import; verify uncertain publication and unchanged SQLite source/data fixtures.
- [x] 2.3 Carry explicit formats through revision-pinned export and bump CLI export version; verify mixed-format JSON/Markdown output and current forget/purge behavior.

## 3. Provider and presentation adapters

- [x] 3.1 Adapt native provider and demo requests/completions to typed calls/results while retaining actor-local Pending; verify both-route HTTP continuation fixtures, ordering and unsupported content refusal before dispatch.
- [x] 3.2 Adapt CLI/TUI/inspection projections and fixtures; verify real text conversation, completed retry, PTY presentation and explicit structured-content export.
- [x] 3.3 Update architecture, protocol and memory/development docs for the typed API and forward-only schema/export compatibility; verify docs build/content checks.

## 4. Integrated verification and delivery

- [x] 4.1 Run relevant granular checks and one combined coverage gate, retaining native CI coverage of memory and terminals; record observed evidence in verification.md and preserve all required thresholds.
- [x] 4.2 Resolve independent review findings and validate the archive-ready implementation with observed local acceptance; record remote CI as a delivery gate after push rather than claim it passed before execution.
