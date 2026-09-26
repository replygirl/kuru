## 1. Characterize today's classification

- [x] 1.1 Add one sample per `ServiceCall`, `ViewOperation` and `LedgerOperation`
  variant (every optional field populated) and a table test pinning each
  variant's serde tag, `may_mutate()` and `unit_receipt_method()`; verify it
  passes against the unmodified functions.

## 2. Exhaustive operation contract

- [ ] 2.1 Add `OperationContract`, `Mutation`, `Receipt` and `ReplyBudget` with
  wildcard-free `contract()` on `ServiceCall`, `ViewOperation` and
  `LedgerOperation`; verify the table test still passes.
- [ ] 2.2 Rebuild `may_mutate`, `unit_receipt_method` and `receipt_progress_key`
  on the contract, share the unit-receipt view helper with `dispatch`, and use
  the reply budget for the attached reply read; verify the invariant test and
  existing receipt tests pass.
- [ ] 2.3 Add `AttachmentState::holds_handles()`/`holds_resources()` at the
  retirement, dream-lease and selected-abandon sites with today's predicates;
  verify retirement and abandonment tests pass.

## 3. Protocol pin and docs

- [ ] 3.1 Add the wire-surface fixture and pin test with serde-derived coverage
  and a refusing bless path; verify it passes at 1.7 and fails on a temporary
  wire edit without a bump.
- [ ] 3.2 Document the protocol-change procedure in `docs/development.md`.

## 4. No-behavior-change verification

- [ ] 4.1 Run the full `kuru-memory` package test task, clippy `-D warnings`
  and format check; verify existing tests pass unchanged and record evidence in
  `verification.md`.
