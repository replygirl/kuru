# Verification

Authored 2026-10-03 before the fix; rows are ticked only when evidence is observed.

## 1. A replaced endpoint record is a miss, not a failure [critical]

- [ ] 1.1 @regression (agent) `EndpointRecord::read_then` with a `between` hook that publishes a private successor record over the name -> `Ok(None)`, then the next `read` returns the successor's record; fails on the unfixed tree with `read private memory service endpoint` caused by `file name no longer identifies the held object` -> evidence pending
- [ ] 1.2 @regression (agent) `service::tests::crashed_owner_retains_accepted_receipt_after_sibling_write` repeated on macOS -> every run passes; PR #205 job 111349259448 and the 2026-10-02 local run are the failing baseline -> evidence pending
- [ ] 1.3 @unit (agent) kuru-platform test on `Directory::verify` -> a replaced name yields the typed outcome with `PermissionDenied` and the unchanged message; a privacy failure on the replacement does not yield it -> evidence pending

## 2. Denials are unchanged [critical]

- [ ] 2.1 @integration (agent) `read_then` with a `between` hook that replaces the record with a non-private object -> the denial still raises (not `Ok(None)`) -> evidence pending
- [ ] 2.2 @integration (agent) a privacy denial on the record still raises through `read_then` -> evidence pending
- [ ] 2.3 @integration (agent) the kuru-connectors and kuru-memory package tests that call `Directory::verify` -> pass with the same kind and message observed through `?` -> evidence pending

## 3. Static checks and native coverage

- [ ] 3.1 @unit (agent) `format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check`, `cospec -- validate --all --strict` -> each exit 0 (`NODE_OPTIONS` unset for docs steps) -> evidence pending
- [~] 3.2 @runtime (agent) native Unix and Windows execution of the new tests -> defer: PR CI runs after push; local macOS evidence is recorded separately and Windows and Ubuntu are not run locally
