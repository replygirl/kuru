## Why

An independent review of PR #89 found that a pre-turn hook rewrite hid the original user input from the model only in the rewritten turn's own messages. The public transcript keeps the original input, which is correct for the user-facing record. The reference visibility policy, however, projects that transcript into every later request's `instructions`. The next turn, a retry of the same turn after an interruption, a resumed session and a fork therefore all sent the original text back to the provider.

A redaction hook would leak the secret it removed on the very next turn. The model would also see a history that contradicts what it actually received: the original prompt followed by its answer to the rewritten one. The archived `lifecycle-hooks` scenario "Provenance record is never sent to a provider" claimed that no provider request carries the original input. That claim was false.

Three smaller review items are in the same area:
- **Omission loop.** The ordinary-request omission loop removed rows from a private list that still held provenance records, while the request builder filtered those records out. Dropping one wasted an estimate iteration and over-counted `omitted_private_rows`.
- **Cleanup test.** The connector test `cancellation_after_root_exit_stops_the_held_output_drain` only checked that `quiesce` returned Ok. It did not assert that the post-exit tail finished well inside the cleanup bound, so it could not detect a regression of that fix.
- **Windows guidance.** The Windows stock PowerShell guidance in `docs/configuration.md` cited CI observations where behavioral advice alone belongs.

## What Changes

- **The model-facing view matches what the model received.** Every provider projection of a rewritten turn's user input uses the final rewritten text, never the original. This covers:
  - the rewritten turn itself and a retry of it
  - every later turn's public-transcript projection in `instructions`, for every actor, including actors that did not take part in the rewritten turn
  - resumed sessions and forks that inherit the turn
  - context compaction
- **One turn-scoped rewrite record.** Before any dispatch, the runtime stores one durable record for the rewritten turn. It contains the rewritten input, hook ordinals and identities, keyed by the turn's primary public node. It holds no original text. The actor's public-transcript projection substitutes it for that turn's user entry.
  - The kuru-hook-linked private row cannot serve every reader. Actors that did not participate have no such row, and forks keep private rows on the parent session.
  - A marker or omission was rejected because it would re-expose hook involvement to the model, which the previous change deliberately hid.
- **User-facing record unchanged.** The public transcript, `history()`, session export and TUI keep the original input. Hook provenance stays durable in the private `kuru-hook` record and the turn-scoped rewrite record; neither is projected to a provider. Turns without a rewrite behave as before.
- **Omission loop.** Provenance records are removed from the private context once, when it is read, without counting them as omitted. Each omission step therefore drops a row that the request actually carried.
- **Cleanup test.** The test asserts that `quiesce` completes in well under `CLEANUP`.
- **Docs.** The Windows guidance is trimmed to its behavioral advice: set `timeout_ms` for stock Windows PowerShell hooks. The configuration and protocol docs state the projection semantics precisely.

## Capabilities

### New Capabilities

### Modified Capabilities

- `lifecycle-hooks`: a pre-turn rewrite replaces the original in every provider projection of that turn's input, including later turns' public-transcript projections, retries, resumes and forks. The user-facing public record keeps the original.

## Impact

- `packages/kuru-runtime/src/engine.rs`: stores the turn-scoped rewrite record before dispatch and adds the key helper.
- `packages/kuru-runtime/src/actor.rs`:
  - the public-window projection substitutes the rewrite record's input
  - provenance filtering moves to context read time
  - `Work` gains the scope
- Tests: `hook_platform_tests.rs` and `hook_tests.rs`, plus `packages/kuru-connectors/src/hooks.rs` for the tightened cleanup assertion.
- Docs: `docs/configuration.md`, `docs/protocols.md` and `apps/kuru-docs/reference/configuration.md`.
- No schema migration: the record uses the existing state store. No dependency changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
