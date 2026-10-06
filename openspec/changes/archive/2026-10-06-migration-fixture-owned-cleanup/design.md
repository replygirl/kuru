# Design

## Context

The fixture's early Result return is not Rust panic unwinding. Its root's Drop
therefore raises a new quiescence panic before libtest can report the original
error. A dropped opening JoinHandle detaches its task; success-only cleanup
does not establish supervisor reap. The historical original error was not
retained in the uploaded evidence and remains unknown.

## Decisions

Retain fixture ownership outside the observed async body. Await the original
opening handle by reference, remove a completed handle before propagating its
inner error, and resume then await an unfinished handle within its existing
migration observation budget during teardown. Only a deadline-expired opening
is aborted and awaited; its cleanup failure remains an error, and root release
preserves any unproven root. Resume fixture pause channels as necessary, close the existing proxy and returned
stores, and await exact project/staging lifecycle quiescence with existing
helpers and budgets. Capture body panics only to execute that same teardown;
preserve their original payload after teardown. Release the root through its
existing Result-preserving accessor after cleanup.

The deliberate-error acceptance uses the same fixture and its real paused
opening/proxy. No generic cleanup framework, registry or test runtime is added.

## Risks / Trade-offs

Cleanup can fail independently; retain its evidence alongside the original
Result and preserve the root rather than weaken the guard. The unchanged
successful lost-commit assertions still prove one durable schema commit and
no replay on reopen. A passing local fixture does not identify the lost
historical error or establish remote CI success.

The first local deliberate-error run preserved its original error but failed
the root guard after immediate opening cancellation: the lifecycle lease was
released before the creator's reaper report arrived. Resuming the same pending
opening lets normal close observe the actual owned supervisor. No additional
ledger waiting mechanism or exhaustive timeout acceptance is introduced.

## Operational surface

Only package-owned local tests change. The fixture uses an isolated private
temporary project scope, prepared pinned full Dolt and a loopback fake SQL
proxy on an ephemeral port. No provider, credential store, external service,
new binary/tool version or new target architecture is involved. Existing
startup/query/reap bounds, platform lifecycle location and task preparation
remain authoritative; no timeout increases or broad test suite are required.
