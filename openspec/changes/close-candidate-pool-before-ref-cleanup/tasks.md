## 1. Candidate pool transition ordering

- [x] 1.1 Close the candidate branch pool with the existing deadline only after promotion is committed or abandonment preflight is complete, before ref cleanup
- [x] 1.2 Preserve the live main pool, transition receipts, conflict usability and non-forced cleanup behavior

## 2. Regression coverage

- [x] 2.1 Add real-Dolt retained-clone and injected post-commit cleanup regressions that fail under the prior ordering and prove exact cleanup retry, close/reopen and undo after the fix
- [x] 2.2 Add retained-clone abandonment and conflict-retention assertions

## 3. Verification

- [x] 3.1 Run focused real-Dolt candidate/runtime checks, formatting, lint, typecheck and strict Cospec gates
- [ ] 3.2 Record hosted Windows memory/runtime acceptance before archive
