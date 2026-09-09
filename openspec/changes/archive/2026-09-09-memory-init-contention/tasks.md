## 1. Diagnose and repair

- [x] 1.1 Add a controlled SQLite lock regression and observe it fail against the original initialization.
- [x] 1.2 Add bounded handling only for journal-transition SQLITE_BUSY while preserving validation and durability.
- [x] 1.3 Verify independent-process initialization and bounded/error paths.

## 2. Validate

- [x] 2.1 Run core behavioral checks, formatting, strict Clippy and coverage; record observed evidence.
- [x] 2.2 Validate and archive the completed change through cospec.
