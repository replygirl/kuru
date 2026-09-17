## Why

The Windows-only regression fixture added with the denied private-stage delete reconciliation never ran before it reached native CI, and it does not reproduce the state it claims. It holds the blocking child with a single `FILE_FLAG_DELETE_ON_CLOSE` handle opened with `FILE_SHARE_DELETE`, so Windows still admits every later checked open: the child delete completes, and the first recoverable error is instead the still-occupied parent directory. `Directory::remove_tree` therefore reports `Uncertain`, having already removed something, and `files::tests::private_temp_retries_a_denied_child_delete_after_the_holder_releases` fails its `assert_eq!` with `left: Some(Uncertain)`, `right: Some(Rejected)` — identically in all five stacked Windows coverage runs, which is a deterministic fixture defect and not a timing race. That single assertion is the only remaining failure in the `native-tests (windows-2025) / Windows coverage (memory-runtime)` job; the three originally failing tests it accompanied now pass, so the reconciliation predicate it was written to cover is correct and only its fixture is wrong.

## What Changes

- Make the fixture leave the child genuinely delete-pending: a retained handle keeps the name in the namespace while a separate delete-on-close handle applies the disposition as it closes, so every later checked open is refused with native error 5 and the checked removal is rejected before anything is removed.
- Assert that denied open inside the fixture, so a change in these native semantics fails at the fixture naming its own premise instead of at the outcome assertion, and explain in the outcome assertion what an uncertain result would mean instead.
- Report the reconcile attempt count and the actual elapsed window in the bounded cleanup exhaustion error, so a future native failure distinguishes one blocked native call from a fast spin without another Windows CI cycle.
- Keep the recovery predicate, the two-second bound, the retry spacing, the exact outer/child identity checks, the no-mutation pending waits and the first-cause error on exhaustion unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/files.rs`: the Windows-only `delete_pending_private_file` fixture and its regression test, plus the added attempt/elapsed context on `wait_for_cleanup_retry` and the counter threaded through `close_windows_private_stage_with`. No platform API, public setting, deletion policy, recovery predicate, publication retry, process lifecycle or timeout change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
