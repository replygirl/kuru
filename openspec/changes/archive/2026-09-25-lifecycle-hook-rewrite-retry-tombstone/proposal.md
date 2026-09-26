## Why

An independent re-review of the archived `lifecycle-hook-rewrite-public-projection` change (051fe68d) found two defects.

The first is a stale rewrite record. A turn can stop after its pre-turn rewrite record is retained but before provider dispatch; the journal then has no possible dispatch, so the turn is retryable. If the retried attempt's hooks no longer rewrite the input, the old `{scope}/pre-turn-rewrite/{node}` record stays in place. Later public-transcript projections then show rewritten text that no model received, while the retry actually sent the original. That contradicts the rule that the provider-facing view matches what the model received.

The second is a flaky test introduced by this PR (2dca3016). `hook_tests::parallel_post_hooks_settle_independently_but_rejoin_in_original_call_order` failed 3 of 20 isolated runs:
- The test asserted a tool settle order.
- That order is the completion order of the parallel wave, and the only thing separating the two completions was a hook-exit gap no wider than the host's 10 ms poll.
- The runtime promises rejoin order, not settle order.

The previous change's verification record wrongly attributed the failure to something outside the change.

## What Changes

- **Stale record cleared on retry.** A re-admitted turn that is not rewritten writes a `cleared` tombstone for its node before dispatch. The tombstone has format, identities and `cleared: true`, and no input. The actor's projection treats a matching tombstone as "use the original user entry". Only re-admitted turns write it: re-admission is rare, and a first attempt cannot have an earlier record. Ordinary turns therefore do no extra reads or writes.
- **Fail-closed diagnostics.** The fail-closed errors in the projection now name the state key, turn ID and node ID.
- **Deterministic parallel test.** The test now releases the first call's post hook only after it observes the runtime's `ToolSettled` event for the second call. The settle order is then fixed by the fixture rather than by timing.
- **Docs and spec wording.**
  - The session export (`kuru sessions export`) keeps the original input.
  - A full `kuru memory export` carries the turn-scoped record and the private `kuru-hook` rows (the rewritten text), but never the original.
  - A retry projects whatever its latest attempt sent.
- **Deferred follow-up (lead).** The public window still performs up to 16 point `get`s per actor request. A batched multi-key read is a separate follow-up and is not part of this PR.

## Capabilities

### New Capabilities

### Modified Capabilities

- `lifecycle-hooks`: rewrite projection follows the turn's latest attempt, the retry-without-rewrite case projects the original, and export wording distinguishes session export from memory export.

## Impact

- `packages/kuru-runtime/src/engine.rs`: tombstone on the non-rewrite re-admission branch.
- `packages/kuru-runtime/src/actor.rs`: tombstone handling and error context.
- `packages/kuru-runtime/src/hook_tests.rs`:
  - a new regression test for the stale retry
  - a deterministic parallel post-hook fixture
- Docs: `docs/configuration.md`, `docs/protocols.md` and `apps/kuru-docs/reference/configuration.md`.
- The verification Notes in the archived `2026-09-25-lifecycle-hook-rewrite-public-projection` record are corrected for the flake attribution.
- No schema migration and no dependency changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
