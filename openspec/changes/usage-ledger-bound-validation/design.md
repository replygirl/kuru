# Design

## Context

Source: `tmp/roadmap/unit7-usage-scan-design-2026-10-01.md` (design sections 1, 2.B, 2.D, 4, 5, 6 and the lead decisions) and `tmp/roadmap/unit7-b-reanchor-2026-10-01.md` (code anchors at origin/main `d17dfe40`). Labels: measured means observed by running, code means read from source, inferred means unchecked reasoning.

- `establish` (`store/usage_ledger.rs`) runs once per writable open under the write guard and walks every owned row twice (scan 1 before `upgrade_usage`, scan 2 after `validate_usage`), then publishes `shared.usage_pool`.
- The walk is linear since `usage-scan-index-range`: measured on Ubuntu CI run 36915490039, 45.8 ms at 4,000 rows and 169.9 ms at 20,000 rows, a slope of 7.76 microseconds per row and a flat part of about 14.8 ms (derived arithmetic).
- Validity is row-local; the ledger never deletes (no `DELETE FROM state` exists); all writes go through `apply_change`'s seven `put_state_tx` sites (code).
- Dolt 2.3.5 facts (measured, design section 7): `DOLT_HASHOF_TABLE('state')` costs 0.3 ms flat, sees the transaction's own writes, ignores other tables, is history-independent and stable across GC; an empty `DOLT_COMMIT --allow-empty` with a message works; `dolt_log LIMIT 1` returns HEAD.

## Goals / Non-Goals

**Goals:**
- A recorded reopen decodes 0 owned rows; the first open after an upgrade or after a foreign change decodes each once.
- Validate-before-activation by construction, with the induction below.
- Missing, foreign or mismatched records never refuse an open by themselves.

**Non-Goals:**
- Incremental validation by `DOLT_DIFF`, flat-check reductions (6b R2/R3), any change to the 30 s deadline, any retry, any read-only open change, any schema or DDL change.

## Decisions

### D1. The induction (design 2.B.2)

Base: S0, the table hash at open, is valid because either the open's full scan validated it or the head record proves an earlier scan or earlier writes did. Step: a write changes only rows it puts, each passed `validate_owned_row` before its `INSERT`; it deletes nothing; validity is row-local, so every owned row in S1 passes the validator. The trailer `V S1` is written in the same commit that creates S1, so no committed state carries an unearned record. A head trailer `V S` with `S` equal to the live hash therefore proves the scan's predicate holds for that content.

Three invariants, stated in the module docs and each tested:
1. **Row-locality.** A cross-row check added to the scan requires bumping the validator stem and redoing this argument; so does any change to the row validator's inputs.
2. **Same function.** Writes call `validate_owned_row` (from `put_state_tx`, after encoding and the 64 KiB cap, before the statement), not a lookalike.
3. **No other writer of owned rows.** Migration publication (state untouched), template adoption (`kuru_instance` only), and foreign or manual commits carry no valid trailer and fall to the scan.

Rejected: trusting the trailer alone (a copied or amended trailer is caught only by the hash comparison); a commit-hash key (self-referential); a row, table, `operations` row, tag or file as carrier (self-referential, schema-version blast radius for every older binary, immutable-receipt meaning, or not atomic with the commit). Design section 2.B.1 holds the full comparison.

### D2. Record carrier and format

Trailer in the ledger's own commit message, same `DOLT_COMMIT` as the writes: `usage ledger v1 [<op>]`, blank line, `Kuru-Usage-State: <validator id> <hash>`. The record commit uses subject `usage ledger validation v1`. The hash is read after the last put and before `DOLT_COMMIT` (measured: sees the transaction's own writes; independent of the `operations` insert). Reader: `DOLT_HASHOF('HEAD')`, then `SELECT commit_hash, message FROM dolt_log LIMIT 1` requiring `commit_hash == HEAD` (never the `dolt_log()` table function, which ignores `-n`, measured); message at most 512 bytes, exactly one trailer line, validator ASCII at most 128 bytes, hash exactly 32 characters of `[0-9a-v]`. Bound(S) when validator equals `VALIDATOR` and hash equals the live hash; every other outcome is Missing.

`VALIDATOR = concat!("kuru.usage.state.v1+", env!("CARGO_PKG_VERSION"))` (`kuru.usage.state.v1+0.9.0` today, code). The release version makes each release boundary cost one linear scan. A golden corpus beside the constant (one accepted row per class; refused near-misses: unknown field, wrong key, unknown class, non-UTF-8 key, zero sequence, bad format, over-long id) is a tripwire for development builds sharing a version string; a flipped verdict requires updating the corpus and bumping the `v1` stem together.

### D3. Write precondition

In `apply_change`, before any read or write in the transaction, read `DOLT_HASHOF_TABLE('state')` and require it equal `shared.usage_validated`. On inequality: roll back, clear the in-memory hash, return a typed error "usage ledger state changed outside its writer since validation; reopen to revalidate". Later writes see no hash and refuse the same way until a reopen. The in-memory hash is set only after `COMMIT` returns. Writes that change nothing roll back and leave it unchanged. Rejected: proceeding as today (an unvalidated foreign row could be built on); re-validating in place (a change under the exclusive lease is an anomaly, so fail closed per "reconcile before further mutation").

### D4. Open sequence with D

Under the guard, after `dolt_status` is clean and the existing flat checks pass: bound check; Bound gives S0 with no rows decoded, otherwise the full scan with the range query using `validate_owned_row`, S0 read under the same clean state, and its count noted. Then `upgrade_usage`, `validate_usage`, and D: re-read the hash S1; if S1 equals S0 no walk, else the full walk. The flat checks of the old scan 2 (`dolt_status`, `validate_historical`, the old-receipt check) stay after the upgrade (about 15 ms measured as the flat part), so only the owned walk is dropped. Then record, then `usage_validated = S1`, then publish the pool.

Record rule: write when HEAD's trailer is not `VALIDATOR S1` and at least one owned row exists. On the full path the count is the scan's own; on the Bound path (a migration commit now sits on HEAD) a plain `SELECT 1 FROM state WHERE key >= ? AND key < ? LIMIT 1` probe (no `FOR UPDATE`, no transaction needed). An empty ledger writes nothing, which keeps the template and reopen tests that pin the usage head and "one commit on the usage branch" true.

Timeline: `establish` still stamps `usage-scan-1` and `usage-scan-2` exactly once each (the job's parser requires it) and calls `usage_rows(0)` on the Bound path and the real count on the full path. The record commit is stamped after `usage-scan-1` so it stays out of the asserted interval. If a D-forced rescan needs a visible count it goes in an additive `Counts` field; format 1 and the parser (no `deny_unknown_fields` on the raw structs) stay unchanged.

### D5. Uncertain outcomes

The anchor in `store.rs` is `resolve_uncertain`, called by usage writes, main writes, candidate operations and `reconcile()`; a usage Pending is otherwise indistinguishable from a main `Receipt::Operation`. Decision: a separate `Receipt::UsageOperation(String)` arm with the same reconciliation as `Operation`, plus the usage-write re-derivation, and `Receipt::UsageValidation { base_head, state_hash }` for the record commit: HEAD equals `base_head` means not committed; HEAD's message parses to `VALIDATOR state_hash` with sole parent `base_head` means committed; anything else (HEAD still at `base_head`, or a head that diverged from both) is not proven and settles as not committed, never as an error. That is safe because reconciliation first waits for the original SQL session to end, so the empty record commit can no longer land, its outcome changes no row and nothing replays it; a missing record only means the next open scans and records again (Missing never refuses). This departs from the roadmap design's 2.B.4 ("anything else: the existing ambiguity error"), which would have left a Pending that blocks every later writer until a reopen. After either arm settles, `resolve_uncertain` re-derives `usage_validated` from HEAD's trailer (Bound against the live hash sets it, anything else clears it), so a committed-but-unanswered write never turns the ledger read-only and the re-derivation happens for every caller. Rejected: `Arc::ptr_eq` on the usage pool (unset during `establish`); re-deriving only in `UsageLedger::change` (misses `reconcile()` and other callers).

The record commit: a definite SQL failure (not uncertain) is logged and the open proceeds unrecorded, since the scan already validated the content (Missing never refuses). An uncertain outcome is resolved inline under the same guard and either settled outcome keeps the open. A resolution that itself fails (the original SQL session does not end within its deadline, or a query fails) still returns that error and refuses the open: that is the existing reconcile-before-mutation rule, not ambiguous evidence. A detected change of HEAD or `state` before the record commit also refuses the open ("state changed while it was being validated").

### D6. What an older binary can do with a ledger written by the new one

It can do everything it could before. Read from the older code (`0.9.0`, design anchor section 7; not yet checked by running an older binary):
- The trailer is a message line in commits whose subjects are unchanged for ledger writes; the validation commit is empty. No DDL, no schema version change, no new table, no receipt format.
- Nothing in older code parses a usage commit message outside the template shape check (`migrations/template_shape.rs`), which runs only on template builds and copies before any ledger write, so it never sees a ledger-written branch.
- Older migration classification reads `dolt_log` and `dolt_commit_ancestors` by commit hash only and concerns attempt heads, so a validation commit on top of a migration head changes no result.
- Its writable open runs its own full scan on every open and its writes ignore HEAD's message, so it validates and writes exactly as before. Its commits carry no trailer, so a newer binary pays one scan and re-records afterward. This is sound in both directions.
Acceptance runs the `0.9.0` release binary, or an origin/main build, against a ledger written by this change once and records it (verification 3.3).

## Risks / Trade-offs

- [Validator change within one version string that the corpus does not cover] → the corpus tripwire, and the rule that any change to `validate_owned_row` or the kuru-core validators it calls bumps the stem.
- [Deliberate forger with database write access writes a matching trailer] → not defended, and the present scan does not defend against that adversary either (it passes well-formed false rows).
- [Dolt serialization change on an engine bump changes table hashes] → the open reads Missing, scans once per project and re-records; note it in the Dolt-bump checklist in `docs/development.md`.
- [Hash collision of a 160-bit hash] → negligible (inferred).
- [A write that finds `state` changed now refuses where it used to proceed] → typed error, reopen revalidates; covered by the foreign-change test.
- [First open after an upgrade pays one full scan] → measured linear since the archived range-paging change; verification 4.1 measures it end to end at 20k conversations.
- [CI calibration] → see D7.

### D7. CI calibration arithmetic (to be fixed with a fresh job run)

Inputs, measured on run 36915490039: T(4,000) = 45.8 ms, T(20,000) = 169.9 ms; derived slope (169.9 - 45.8) / 16,000 = 7.76 microseconds per row, intercept 45.8 - 4,000 x 0.00776 = 14.8 ms. Predicted linear T(20,000) = 14.8 + 155.2 = 170 ms. Unfloored ratio 3.71 (below the ideal-linear 5.0 because of the flat part); quadratic growth gives at least the measured pre-range ratio of 19.6, extrapolated higher at these sizes.
- K stays 8: it sits between linear (3.71 measured, 5.0 ideal) and quadratic (at least 19.6), about 2.2 times the measured ratio. The 100 ms floor stays (the 4,000-row sample is below it), making the ratio check effectively T(20,000) at most 800 ms, 4.7 times the measured 169.9 ms.
- Ceiling: 8 x 169.9 ms = 1,359 ms, rounded up to 1,500 ms, a margin for a slower runner and about 20 times under the 30 s deadline (replacing 3,000 ms).
- Bound sample (new, applies to the Bound series only): `usage-pool` to `usage-scan-1` at most 50 ms at both sizes, about 3.4 times the measured 14.8 ms flat part, and `counts.usage_rows == 0` at both sizes.
The implementation re-derives these numbers from the job's printed rows in its PR run and states the final arithmetic in verification; if the observed values differ materially the bounds follow the same formulas.

### D8. Driver forcing

The driver never forced the full path (code, re-anchor 3): after this change the aged fixture is Bound. Before each full sample the driver commits an empty foreign commit on the usage branch through a released server (so HEAD carries no trailer), then takes a bound sample after the following open records. Samples split into a full series (asserted by the ratio and ceiling) and a bound series (asserted by 0 rows and the 50 ms bound). The `rows` check and `gated_opens_of_sealed_aged_stores_count_the_planned_rows` are adapted to expect planned rows on forced-full samples and 0 on bound ones.

## Open Questions

- Whether a D-forced rescan needs its own `Counts` field is decided during implementation; it does not change the specs, the approach or the tasks.

## Operational surface

The only deploy-topology change is the existing `usage-scan-scaling` job in `.github/workflows/ci.yml`: it stays on `ubuntu-latest`, `needs: bundle-inputs`, 45-minute timeout, exact-key fixture restore, main-only save, and stays in `ci-gate`'s `needs`. No new secret, no bind address, no container, no connection-limit change. The Dolt binary is the pinned 2.3.5 bundled engine (`support/dolt-assets.json`); the job is Ubuntu only and the trailer and hash behaviour is unverified on Windows locally (native CI runs the ordinary suite). The fixture cache key does not cover the write path or the validator, so a fixture cached before this change restores unchanged and its first open reads Missing; the driver's forcing step (D8) keeps the measured path independent of that. The cache-budget PR may overlap textually in the job's cache steps.

## Integration contract

The external contract is Dolt 2.3.5 SQL: `DOLT_HASHOF_TABLE('state')`, `DOLT_HASHOF('HEAD')`, `dolt_log` (first row, checked equal to HEAD), `DOLT_COMMIT('--allow-empty', ...)` and `dolt_commit_ancestors` for the record arm's parent check. Their behaviour is pinned by the measured probes in design section 7 of the roadmap design (flat cost, sees own writes, history-independent, GC-stable) and by tests that run against real Dolt; an engine bump can change table-hash values or plan text, which reads as Missing and costs one rescan. Nothing here changes a schema, id type or version: the usage branch keeps `USAGE_REGISTRY` `[V2, V3, V4]`, the state table, and `USAGE_CURRENT_VERSION` 4, and the receipt format is unchanged. The commit-message trailer is the only new on-disk artifact, and its format is specified in the `Usage validation record convention` requirement.
