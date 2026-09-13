## 1. Durable notice adapter

- [x] 1.1 Add the private project-scoped version adapter over existing `MemoryStore::status`, `get`, and reconciled `put`, and verify pending/current/malformed state behavior with focused memory-backed tests.

## 2. Application presentation

- [x] 2.1 Wire headless runtime owners, including provider-free undo, to flush stderr before recording while preserving stdout, and verify isolated CLI JSON/reopen/no-provider behavior.
- [x] 2.2 Preserve public `ui::run`, add private optional-notice UI wiring after the first completed draw and before input, and verify real PTY plus failing-draw/no-model-history fixtures.

## 3. Documentation and verification

- [x] 3.1 Document the informational notice and exact memory controls in owning memory pages, and verify docs checks.
- [x] 3.2 Run focused app/memory format, typecheck, lint, and behavior checks; record observed local, coordinated-coverage, and native Windows evidence.
