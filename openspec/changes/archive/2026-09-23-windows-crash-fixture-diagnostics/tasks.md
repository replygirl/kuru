## 1. Windows crash fixture diagnostics

- [x] 1.1 Update `packages/kuru-memory/src/service.rs` to distinguish the last missing-endpoint and published-transport observations while retaining bounded child and Dolt startup diagnostics under the existing deadline.
- [x] 1.2 Add focused source-level coverage for bounded diagnostic capture and run the relevant `kuru-memory` typecheck/test checks without claiming native Windows cause evidence.

## 2. Delivery

- [x] 2.1 Record the observed main Windows failure and local verification, archive the test change, and commit the isolated correction for hosted native diagnosis: main `b3b8965` timed out after 20 seconds with the child still running; all-target typecheck and both bounded diagnostic tests pass locally; Windows native cause evidence remains pending hosted CI.
