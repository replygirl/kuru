# Tasks

## 1. Diagnosis record

- [ ] 1.1 Record, from PR #133 run 36998368110 / job 110810405623, the exact
      panic text, the unexplained store path (the project directory itself,
      not a `.staging-` sibling or an `interrupted/` stage), and that 158
      other real-engine fixtures in the same partition passed, and verify by
      reading the fetched job log (`gh api --allow-escape-sequences
      repos/replygirl/kuru/actions/jobs/110810405623/logs`).
- [ ] 1.2 Trace and record the successor election path
      (`attach_or_spawn_elected`'s `ServiceLock::try_acquire(Owner)` wait,
      then `Server::open`/`open_inner` taking the project's lifecycle lease)
      to confirm the successor cannot open while the crashed owner's
      lifetime-pipe supervisor still holds that lease and is still reaping
      Dolt, and verify by citing the exact functions/line ranges read in
      `packages/kuru-memory/src/service.rs` and `src/server.rs`.
- [ ] 1.3 Conclude and record whether the leak is in the fixture's exit-path
      coverage (missing `await_managed_quiescence`/`release` on early
      returns and on `timeout_at` cancellation) or in the product's crash
      path, citing 1.1 and 1.2; verify by checking the conclusion against the
      non-goals (no change proposed to `attach_or_spawn_elected`,
      `ORDINARY_POOL_WINDOW`, or the supervisor readiness deadline).

## 2. Deterministic regression test

- [ ] 2.1 Add a fixture that spawns a real owner via `spawn_service`, SIGKILLs
      it, injects an early `bail!` before the body's existing final
      `await_managed_quiescence` call, and asserts the surfaced error is the
      injected failure text, not the fixture-root guard's "no quiescence
      record" message; verify by running it before the fix and observing it
      panic with the guard's message instead of the injected one (failing as
      intended), named in the record with the exact command used.
- [ ] 2.2 After the restructure (task 3), verify the same new fixture passes:
      the injected error surfaces with the guard's kept-root note attached as
      context only if quiescence could not be awaited, and the fixture root
      is actually released once the killed owner's engine quiesces; verify
      by running `mise run //packages/kuru-memory:test -- service::tests::` and
      recording the observed pass, or naming why it could not be run.

## 3. Fixture restructure (test-only)

- [ ] 3.1 Move the `TempDir` in
      `crashed_owner_retains_accepted_receipt_after_sibling_write` outside
      the fallible `async` block so the outer `tokio::time::timeout_at` wraps
      only the body, not the root; verify by reading the diff and confirming
      `root` is no longer moved into that block.
- [ ] 3.2 Replace the implicit drop at the end of the test with
      `root.release(outcome)`, where `outcome` folds the body's `Result` and
      the `timeout_at` elapsed case into one `anyhow::Result<()>`, matching
      the `served_owner` contract (`a_body_error_is_returned_first_with_a_teardown_error_attached`)
      so a body error is returned first and any teardown violation is
      attached as context, never replacing it; verify by reading the
      resulting control flow against that existing test's contract.
- [ ] 3.3 Confirm `test_support::retire_idle_service` (called inside
      `await_managed_quiescence`) does not itself error when the owner
      already died without publishing a current endpoint, so a best-effort
      quiescence attempt after an early failure does not mask the body's
      error with an unrelated one; verify by reading its handling of a
      missing/stale endpoint record.
- [ ] 3.4 Apply the identical restructure to the new fixture from task 2;
      verify by reading both tests' final shape side by side.

## 4. Evidence collection

- [ ] 4.1 Run the restructured `crashed_owner_retains_accepted_receipt_after_sibling_write`
      and the new fixture locally via the package's real-engine test task
      and record pass/fail with the exact command and duration, or name
      explicitly why a run could not be completed (e.g., shared build cache
      contention) rather than asserting an unrun pass.
- [ ] 4.2 Confirm no other test in `packages/kuru-memory/src/service.rs` that
      relies on `TempDir` without `.release(...)` shares this exact
      crash-then-elect shape, so this fix's scope is not silently
      incomplete; verify by listing every `#[tokio::test]` in the module that
      spawns a real owner process and recording which already call
      `await_managed_quiescence`/`.release(...)` before their root drops.
