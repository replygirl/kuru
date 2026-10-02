# Proposal

## Why

`service::tests::crashed_owner_retains_accepted_receipt_after_sibling_write`
failed in PR #133's run 36998368110 (macos-latest coverage partition 1, job
110810405623). The fixture-root guard panicked with "fixture root … was
released without awaited memory quiescence … store …/memory/e9e956f6… has no
quiescence record". The test ended at most 21.6 s after its binary started,
while its `fixture_deadline(1, 1)` budget is over 96 s, so its body returned an
early error. The guard's panic replaced that error, and it is not in the log.

The product reaps the crashed owner's engine. The supervisor runs in its own
process group and takes Dolt down on lifetime-pipe EOF. It holds the
lifecycle lease until Dolt is reaped, and the successor's supervisor waits for
that lease. The leak is in the fixture. Its root and its single
`await_managed_quiescence` both lived inside the timed body, and quiescence
was awaited only on the success tail. Every early `?`, `ensure!`, `bail!` or
readiness return, and an elapsed deadline, dropped the root right after
`KillServiceOnDrop` SIGKILLed the service without waiting. The orphaned
supervisor was then still stopping Dolt. PR #125 named this class
("guarded roots created inside a `tokio::time::timeout` future without serving
an owner") as an open follow-on.

## What Changes

- `packages/kuru-memory/src/service.rs` (tests module only):
  - The crash-then-elect body moves into one fixture function,
    `crashed_owner_receipt_fixture`, with the same steps and assertions. It
    takes an optional injected failure. Its root and options sit outside the
    timed stage, and the stage runs through `FixtureDeadline::run`. After the
    stage, on every exit path (success, body error, elapsed deadline), it
    awaits `await_managed_quiescence` through `settle`, and then releases the
    root with `TempDir::release`. A body error is printed and returned first,
    and any teardown failure or guard verdict is attached as context. The
    success tail no longer calls `await_managed_quiescence` itself, so the
    call still runs exactly once.
  - `crashed_owner_retains_accepted_receipt_after_sibling_write` runs the
    fixture without an injection and covers the same crash-recovery receipt
    contract as before.
  - New `crashed_owner_fixture_failure_is_reported_after_its_engine_quiesces`
    injects a failure after the sibling's write and the owner-liveness check,
    before the deliberate crash, while the owner's Dolt is certainly running.
    It asserts two things. The reported error is exactly the injected text,
    with no teardown error or guard verdict attached. The fixture root no
    longer exists, so it was released and not kept. On the old structure this
    test fails with the guard's panic.
- `packages/kuru-memory/src/test_support.rs`: `settle` joins the existing
  `cfg(test)` re-exports from `served_owner`.

## Impact

- Test-only. No product code, spec, on-disk format or deadline changes.
  `server.rs`, `attach_or_spawn_elected`, `ORDINARY_POOL_WINDOW` and the
  supervisor readiness deadline are untouched.
- The quiescence wait now runs after the fixture deadline's timed stage on
  every path, following the `served_owner` precedent. It keeps its existing
  bounds: 10 s in `retire_idle_service`, `startup_timeout_secs` in the
  maintenance-permit loops, and `SUPERVISOR_REAP_ALLOWANCE` per store
  directory.
- Adds one real spawned-owner fixture (one fresh lifecycle plus the crashed
  supervisor's reap) to the kuru-memory lib suite and its coverage partition.
  The measured time is recorded in `tasks.md`.
- Out of scope: the other guarded roots created inside timeout futures
  (service.rs near 3682, 6008, 6212 and 6489; store.rs near 12818 and 13017).
  They are not implicated in this failure.
