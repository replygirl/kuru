## 1. Pin coverage

- [x] 1.1 Add per-variant samples for the export phase, `UsageProof`,
  `ContentBlock`, `SessionTurnCheckpoint`, `PublicTranscriptPosition`,
  `PriceBasis` and `CacheWriteTerms`, populate `price_at_invocation`, and check
  each set against serde's variant list; verify with the pin test.
- [x] 1.2 Read the fixture at test time and allow regeneration only for an absent
  fixture or a strictly greater version; verify the refusal and acceptance paths.
- [x] 1.3 Regenerate the 1.7 fixture through the tooling and verify it passes.

## 2. Contract path and docs

- [x] 2.1 Replace the `unit_receipt_view` wildcard with explicit arms and use the
  contract budget in the paused exchange; verify contract tests pass.
- [x] 2.2 Update `docs/development.md` for sampled-shape limits and residual
  golden-file risks.

## 3. No-behavior-change verification

- [x] 3.1 Run the full package test task, clippy and format; record evidence.
