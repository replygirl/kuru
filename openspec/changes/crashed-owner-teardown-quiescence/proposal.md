# Proposal

## Why

`service::tests::crashed_owner_retains_accepted_receipt_after_sibling_write`
failed in PR #133's run (36998368110, macos-latest coverage partition 1, job
110810405623): the fixture-root guard (`test_support::fixture_dir`) panicked
at teardown with "was released without awaited memory quiescence … store
…/memory/e9e956f6…1 has no quiescence record". The test ran 158 other
real-engine fixtures clean in the same partition, including several that
specifically exercise crash/reap paths (`owner_retires_at_once_when_its_starter_detaches`,
`rejected_publication_reaps_engine_before_owner_lock_releases`), and the
successor election this test itself performs (`attach_or_spawn_elected`
waiting on the owner lock, then `Server::open`/`open_inner` taking the
project's lifecycle lease) cannot proceed while the crashed owner's
lifetime-pipe supervisor still holds that lease and is still reaping Dolt —
so the successor's later `attach_or_start` returning at all is itself
evidence the product-side reap already completed.

The test calls `test_support::await_managed_quiescence` exactly once, at the
very end of its body, after the successor's maintenance permit is dropped.
Every earlier fallible step (`?` on I/O and RPC calls, every `ensure!`, the
`fixture_readiness_error` early returns, and the outer `tokio::time::timeout_at`
cancelling the body on budget expiry) drops the fixture's `TempDir` without
ever reaching that call. `TempDir`'s own `Drop` impl then runs the same
unexplained-store scan and panics — and because the thread is not yet
unwinding at that point, that panic *replaces* whatever error or timeout the
body was actually returning, so the CI log shows only the guard's "no
quiescence record" message with no trace of the real failure. This is the
same failure the module's own `release(outcome)` helper and the
`served_owner` test `a_body_error_is_returned_first_with_a_teardown_error_attached`
exist to prevent; this fixture does not yet use that pattern.

## What Changes

Restructure the fixture so its `TempDir` is held outside the body's fallible
`async` block and released through `TempDir::release(outcome)` on every exit
path — the happy path, an early `?`/`ensure!` return, and an outer-deadline
cancellation alike — instead of being dropped implicitly. Before `release`
runs, the fixture performs its existing cleanup (killing the owner process via
`KillServiceOnDrop`, which already happens on scope exit) and attempts
`await_managed_quiescence` so a failing run still records quiescence and
frees its temp directory when the owner and any successor have actually
quiesced, bounded by the helper's own `SUPERVISOR_REAP_ALLOWANCE` wait with no
new deadline. A body error is returned first, with any teardown violation
attached as context, matching the `served_owner` contract already proven
elsewhere in this module.

Add a second, deterministic fixture that forces an early failure after the
owner is killed (an injected `bail!`) and asserts: (a) the surfaced error is
the injected failure, not the guard's "no quiescence record" message, and (b)
the fixture root is actually released (not kept) once the killed owner's
engine quiesces. This test fails today (it panics with the guard's message
instead of the injected one) and passes once the restructure lands.

No product code changes: `packages/kuru-memory/src/server.rs`'s crash/reap
path, `attach_or_spawn_elected`, `ORDINARY_POOL_WINDOW` and the supervisor
readiness deadline are untouched.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — the fixture-root quiescence guard's behavior was already correct and
is not changing; this fixes the one test that did not use it on every exit
path.

## Impact

- `packages/kuru-memory/src/service.rs` — restructures
  `crashed_owner_retains_accepted_receipt_after_sibling_write` (test-only) and
  adds one new `#[tokio::test]` fixture (test-only) exercising the same
  crash-then-elect lifecycle with an injected early failure.
- No changes to `packages/kuru-memory/src/server.rs`, `src/test_support.rs`,
  or any other non-test file.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
