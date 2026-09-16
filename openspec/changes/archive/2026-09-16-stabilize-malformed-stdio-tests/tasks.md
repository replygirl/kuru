## 1. Deterministic malformed-peer regressions

- [x] 1.1 Add a trailing read to both malformed stdio plans while retaining their old `.done` assertions and verify both focused regressions fail because the peer cannot naturally complete
- [x] 1.2 Replace the connector malformed-peer completion assertion with exactly one initialize-request transcript per failed peer and verify both alias orders retain the healthy HTTP catalog, unavailable stdio status, and confirmed cleanup
- [x] 1.3 Replace the CLI malformed-peer completion count with exactly four one-request initialize transcripts and verify all existing output, degradation, redaction, event, and cleanup assertions remain intact

## 2. Verification

- [x] 2.1 Run the settled connector and TUI focused regressions and record their observed green evidence in `verification.md`
- [x] 2.2 Run scoped formatting, typecheck, and lint checks and record their observed evidence in `verification.md`
