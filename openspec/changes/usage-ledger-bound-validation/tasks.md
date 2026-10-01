# Tasks

## 1. Row validator and trailer

- [x] 1.1 Factor `validate_owned_row(key: &[u8], value: &str)` out of the scan loop (keep the `session-index/` before `session/` order), call it from the scan and from `put_state_tx` before the `INSERT`, and verify with the existing WP0 refusal tests unchanged plus the same-function test (T14)
- [x] 1.2 Add `VALIDATOR`, the trailer encoder and the strict parser, the golden corpus beside the constant, and the module-doc invariants (row-locality, same function, no other writer), and verify with the parser and corpus tests (T17, T18)

## 2. Writes

- [x] 2.1 Add `Shared.usage_validated` (both constructions), read `DOLT_HASHOF_TABLE('state')` before any read in `apply_change`, refuse with the typed error and clear the hash on inequality, append the trailer from the hash read after the last put, and set the hash only after `COMMIT`; verify with T12 and the adapted `uncertain_committed_receipt_reconciles_without_replaying_usage`
- [x] 2.2 Add `Receipt::UsageOperation` and `Receipt::UsageValidation` with their `resolve_uncertain` arms and the post-settle re-derivation from HEAD's trailer, and verify with T13 (committed and not-committed, for a write and for the record commit)

## 3. Open

- [x] 3.1 Implement `bound_validation` and the new `establish` order (bound check or full scan, `upgrade_usage`, `validate_usage`, D hash re-read, flat checks, record commit when an owned row exists and HEAD lacks the record, set the hash, publish), keeping each timeline stamp exactly once and `usage_rows(0)` on Bound; verify with T9, T10, T11, T15, T16, T20 and T19 unchanged
- [x] 3.2 Confirm the tests that pin an empty ledger writes nothing (`creation_template/open_tests.rs`, `template_stage_tests.rs`, `engine_contract_tests.rs`) and WP0's `plant` tests stay green unchanged, and verify by running them

## 4. CI

- [x] 4.1 Add the forcing step and the full and bound series to the usage-scan driver, adapt `gated_opens_of_sealed_aged_stores_count_the_planned_rows` and the verdict and report tests, enable the 0-rows assertion, and verify with the driver unit tests and the real-engine row-count test
- [x] 4.2 Set the calibrated bounds with the derivation of design D7 (K = 6, floor 100 ms, ceiling 1,000 ms from the four Ubuntu runs; the bound series asserts 0 rows and its timing is printed and not bounded until an Ubuntu run measures it), update the job name, comments and printed labels in `ci.yml` and the `release_workflow` shape test, and verify with `mise run //packages/kuru-delivery:test` and `lint:tooling`

## 5. Documentation

- [x] 5.1 Document the trailer as a durable convention and the one-time re-check after an upgrade in `docs/memory.md` (keep "one commit on the usage branch" true), and the calibrated bounds, bound series and Dolt-bump note in `docs/development.md`, and verify with `mise run docs:check`

## 6. Verification

- [x] 6.1 Run `format:check`, `lint`, `//packages/kuru-memory:lint:windows`, `typecheck`, `lint:tooling` and the kuru-memory and kuru-delivery tests, and record the results
- [ ] 6.2 Record the measurements listed in verification.md section 4 and the older-binary open in section 3, then validate, apply and archive with cospec
