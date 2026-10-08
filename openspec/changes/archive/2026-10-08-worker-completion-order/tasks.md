# Tasks

## 1. Completion ordering

- [x] 1.1 Audit native completion, operation holds and intentional ownership handoffs across connectors, memory, platform and runtime.
- [x] 1.2 Add an event-driven regression that fails on the original result-before-release ordering.
- [x] 1.3 Correct confirmed hook, shell, MCP and session-selection completion without releasing unconfirmed native ownership.
- [x] 1.4 Cover success, clean failure, caller loss and retained cleanup using native fixtures and observable hold/lease state.

## 2. Verification and delivery

- [x] 2.1 Run connector tests, native ownership regressions, host/Windows lint, format and documentation checks; record actual evidence.
