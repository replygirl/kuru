## 1. Classification is unchanged [critical]

- [x] 1.1 @equivalence (agent) run the new classification table test against the pre-refactor `may_mutate`/`unit_receipt_method`, then against the contract-based code -> `every_operation_keeps_its_pinned_classification` passed on the unmodified functions (commit 110a289b) and passes unchanged after the refactor. It covers all 58 leaf operations: 23 direct calls, 30 view operations and 5 ledger operations
- [x] 1.2 @unit (agent) run the receipt-class invariant test -> `only_idle_retirement_mutates_without_a_durable_receipt` and `every_operation_contract_decides_receipt_and_reply_budget` passed. `retire_if_idle` is the only write without a receipt, no read carries one, and every reply budget is `Operation` (35 s)
- [x] 1.3 @equivalence (agent) run the complete `kuru-memory` package test task (real Dolt, managed service, facade receipt/fence and recovery tests) -> `mise run //packages/kuru-memory:test` exit 0. Library: 272 passed, of which 267 existed before and 5 are new. Integration targets: 6, 5, 12 and 1 passed. No existing test body was edited

## 2. Protocol pin [critical]

- [x] 2.1 @unit (agent) run the protocol-pin test at 1.7 -> `protocol_surface_matches_the_pinned_protocol_version` passed. Request samples cover exactly serde's variant lists: 25 `ServiceCall`, 30 `ViewOperation` and 5 `LedgerOperation`
- [x] 2.2 @regression (agent) temporarily change a request field name, then run the pin test with and without `KURU_BLESS_PROTOCOL_PIN=1` at an unchanged version -> a temporary `#[serde(rename = "key_name")]` on `ViewOperation::Get` produced these results. Without bless, the test failed with the `- view_operation.get {key:string}` / `+ {key_name:string}` diff and bump instructions. With bless, it failed with "refusing to record … under unchanged protocol 1.7", and the fixture checksum was unchanged. With the minor temporarily set to 8, it failed with "already 1.8 … regenerate", and bless then rewrote the fixture and passed. All temporary edits were reverted and the fixture checksum was verified
- [x] 2.3 @regression (agent) temporarily add a new view variant with no contract entry -> `ViewOperation::Probe` failed to compile with E0004 at the `ViewOperation::contract` match (and `dispatch_view`). After adding contract and dispatch arms but no sample, the pin test failed with "view_operation samples … must cover exactly the wire variants" listing `probe`. Reverted

## 3. Static checks

- [x] 3.1 @integration (agent) run package clippy with `-D warnings`, formatting check and strict cospec validation -> `mise run //packages/kuru-memory:lint` exit 0, `mise run format:code` exit 0 and `cospec validate --strict` passed
- [~] 3.2 @e2e (human) native Windows and Linux CI for the package -> defer: pure classification code with no platform branch; CI on the branch provides the native evidence and is not inferred locally

## Observed evidence

2026-09-25, native macOS, worktree branch `refactor/memory-rpc-operation-contract`
from `67448442`. Drift check: the git history of `PROTOCOL_MINOR` against
`rpc.rs` shows one unbumped wire addition, `PutReasoningSummaries` and
`ReasoningSummaryConflict` in #81. It shipped in v0.8.0 together with the
bump to 4 in #82, so no released version carried it without a bump. v0.9.0
shipped 1.6. #87 changed the wire and bumped to 1.7. No current drift was
found. The fixture records 1.7 as its baseline.
