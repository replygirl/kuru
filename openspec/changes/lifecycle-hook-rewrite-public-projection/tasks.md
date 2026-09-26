## 1. Rewritten input in every provider projection

- [x] 1.1 Store one turn-scoped rewrite record (rewritten input, hook ordinals and identities, no original) before provider dispatch, keyed by the turn's primary public node, and verify that it is present after a rewritten turn
- [x] 1.2 Substitute the rewrite record's input for that turn's public user entry in the actor's public-transcript projection, and verify with a regression test that fails before the fix. The test covers a later turn to a different actor, compaction, resume and a fork: no request message or instructions may carry the original, and the user-facing history keeps it.
- [x] 1.3 Filter provenance records once when private context is read, so the omission loop drops only projected rows, and verify with the existing hook and context-fit tests

## 2. Test and documentation corrections

- [x] 2.1 Assert that `quiesce` finishes well under `CLEANUP` in `cancellation_after_root_exit_stops_the_held_output_drain`, and verify that the connector hook suite passes
- [x] 2.2 Document the projection semantics in `docs/configuration.md`, `docs/protocols.md` and the docs site reference, trim the Windows guidance to behavioral advice, and verify with `docs:check`
- [x] 2.3 Run focused checks (format, clippy, focused tests, docs), and record evidence and unrun checks in verification.md
