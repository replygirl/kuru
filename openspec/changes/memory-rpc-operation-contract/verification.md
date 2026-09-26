## 1. Classification is unchanged [critical]

- [ ] 1.1 @equivalence (agent) run the new classification table test against the pre-refactor `may_mutate`/`unit_receipt_method`, then against the contract-based code -> the same table of every ServiceCall, ViewOperation and LedgerOperation variant (serde tag, mutates, unit receipt method) passes both times
- [ ] 1.2 @unit (agent) run the receipt-class invariant test -> every mutating call has a receipt class except `retire_if_idle`; no read-only call has one
- [ ] 1.3 @equivalence (agent) run the complete `kuru-memory` package test task (real Dolt, managed service, facade receipt/fence and recovery tests) -> all pass unchanged with no existing test edited

## 2. Protocol pin [critical]

- [ ] 2.1 @unit (agent) run the protocol-pin test at 1.7 -> fixture matches; request samples cover exactly serde's variant lists
- [ ] 2.2 @regression (agent) temporarily change a request field name, then run the pin test with and without `KURU_BLESS_PROTOCOL_PIN=1` at an unchanged version -> both fail with bump instructions and the fixture is not rewritten; revert
- [ ] 2.3 @regression (agent) temporarily add a new view variant with no contract entry -> compilation fails at the exhaustive contract match; revert

## 3. Static checks

- [ ] 3.1 @integration (agent) run package clippy with `-D warnings`, formatting check and strict cospec validation -> all pass
- [~] 3.2 @e2e (human) native Windows and Linux CI for the package -> defer: pure classification code with no platform branch; CI on the branch provides the native evidence and is not inferred locally
