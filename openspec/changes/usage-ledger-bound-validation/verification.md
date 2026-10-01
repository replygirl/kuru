# Verification

Tests are numbered as in `tmp/roadmap/unit7-usage-scan-design-2026-10-01.md` section 4. T1-T8 (WP0 refusals, range paging) are already on main. This change's tests are T9-T20, grouped below; T1-T4b are re-run unchanged as the regression for the shared row validator. Nothing in this ledger has been run yet.

## 1. A recorded ledger reopens without decoding a row, and an unrecorded one is scanned and recorded [critical]

- [ ] 1.1 @integration (agent) T10: build a usage head without a trailer, reopen twice and write once -> reopen 1 decodes the owned row count, adds exactly one commit whose message parses to `VALIDATOR S`, leaves `operations` unchanged; reopen 2 decodes 0 rows and leaves HEAD unchanged; a write then reopen 3 is bound with no record commit
- [ ] 1.2 @integration (agent) T11: a trailer with another validator id gives a full scan then a record; a trailer plus an unknown class present gives a refusal -> as stated
- [ ] 1.3 @integration (agent) T15: a fresh store and a template-born store -> no record commit at open, usage head equals the adoption head, `engine_contract_tests` usage-head checks unchanged
- [ ] 1.4 @integration (agent) T16: a v3 usage branch with owned rows upgraded at open -> one walk (rows decoded once, not twice), one record commit on top of the migration commit
- [ ] 1.5 @runtime (agent) T20 and the CI job -> the timeline records the bound and full path rows (0 versus the owned count) that the job reads

## 2. Corrupt state never activates [critical]

- [ ] 2.1 @integration (agent) T9: after a record, a foreign commit replaces one owned value with malformed JSON, (a) without a trailer, (b) copying HEAD's message with its trailer, (c) with a valid but non-canonically keyed value -> the reopen refuses in every variant
- [ ] 2.2 @integration (agent) T12: a foreign commit changes `state` between open and write -> the typed error, no commit, no `operations` row, later writes refuse, a reopen scans and accepts
- [ ] 2.3 @integration (agent) T14: a scripted session (mark, admit, observe x3, settle) -> every written row passes `validate_owned_row`, and an invalid encode refuses before commit with HEAD unchanged
- [ ] 2.4 @regression (agent) T1-T4b (WP0) -> pass unchanged

## 3. Uncertain outcomes and older binaries

- [ ] 3.1 @integration (agent) T13: lost-reply cases for a usage write and for the record commit (`Receipt::UsageOperation`, `Receipt::UsageValidation`) -> committed: the hash is re-derived and the next write succeeds; not committed: the old hash is kept and the retry succeeds
- [ ] 3.2 @unit (agent) T17 and T18 -> every malformed trailer form reads Missing; the golden corpus verdicts are pinned
- [ ] 3.3 @manual (agent) open a ledger written by this change with the `0.9.0` release binary or an origin/main build -> the older binary opens, validates and writes as before (nothing was checked by running; the older-binary claim rests on reading its code)
- [ ] 3.4 @regression (agent) T19 and the adapted `uncertain_committed_receipt_reconciles_without_replaying_usage` -> reopen retains receipts unchanged; the committed-receipt case writes again

## 4. Measurements and the CI job [critical]

- [ ] 4.1 @benchmark (agent) the first open after an upgrade at 20k conversations, end to end, release build -> not yet run; recorded in the PR body
- [ ] 4.2 @benchmark (agent) age-scaling re-run at 1k, 5k and 20k conversations -> not yet run; scan rows are 0 on a recorded reopen at every size and the first-open row is one linear scan
- [ ] 4.3 @runtime (agent) the `usage-scan-scaling` job on this PR's CI run -> not yet run; full series passes the calibrated ratio and ceiling, bound series shows 0 rows and at most 50 ms, the derivation arithmetic of design D7 is restated with the observed numbers
- [ ] 4.4 @unit (agent) the driver unit tests and the real-engine row-count test -> not yet run; the forcing step gives planned rows on full samples and 0 on bound ones, the 0-rows assertion fails on a nonzero bound sample

## 5. Static checks

- [ ] 5.1 @integration (agent) `format:check`, `lint`, `//packages/kuru-memory:lint:windows`, `typecheck`, `lint:tooling`, `docs:check` -> not yet run; Windows behaviour is unverified locally and only the target lint compiles it
