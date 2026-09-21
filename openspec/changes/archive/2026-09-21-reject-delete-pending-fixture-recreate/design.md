## Context

`remove_fixture_binary` (Windows-only, `packages/kuru-memory/src/provision/native_tests.rs`) already retries the removal call itself across a bounded 2-second window when Windows refuses the delete-capable open or the disposition with native error 5 or 32 — this is the same family of transient native holder already fixed for stage cleanup in `reconcile-denied-private-stage-delete` / `reject-delete-pending-stage-child-fixture`. But once `parent.remove_file(...)` itself returned `Ok(())`, the function returned immediately. Both of its callers then create a fresh file at the identical path (`new_private_file`) to install corrupted fixture bytes.

The removed name is the just-executed `dolt.exe` warm-probe copy. Windows tears down an executed image's section asynchronously after the handle closes; the name can remain delete-pending for a short window even after the `DeleteFile`/`SetFileDispositionInfo`-equivalent call the removal used has itself returned success. A `CREATE_NEW` open at a delete-pending name is refused with `ERROR_ACCESS_DENIED` (native error 5) until the pending delete completes — exactly the panic observed in CI run 35584395348, immediately after the removal had already reported `Ok(())`.

## Goals / Non-Goals

**Goals:** Make `remove_fixture_binary` wait for genuine absence before returning, so its callers' immediate recreate at the same name never races the asynchronous teardown; keep the wait bounded by the window the function already uses; report enough diagnostic detail on exhaustion to distinguish a single blocked native call from a fast spin.

**Non-Goals:** No change to the removal retry's own recoverable-error predicate, to any production (non-test) code path, to `new_private_file`, or to any deletion/publication/process policy. No new magic numbers, no widened product bound.

## Decisions

Fold the wait into `remove_fixture_binary` itself (remove → wait for absence) rather than adding it at each of the two call sites, so both benefit without duplicating the pattern. The wait is a new `wait_for_fixture_binary_absence` helper that polls `fs::symlink_metadata` on the removed path: only `io::ErrorKind::NotFound` counts as absence. A successful stat or a still-denied query are both treated as "not yet absent" and retried — the function never infers absence from anything except `NotFound`, since a successful attribute query on a delete-pending name cannot be relied on to mean the name is gone.

The removal retry and the absence wait now share one deadline (`retry_deadline: &mut Option<Instant>`, threaded from `remove_fixture_binary` into the wait) and one pair of named constants (`FIXTURE_CLEANUP_RETRY_LIMIT` = 2s, `FIXTURE_CLEANUP_RETRY_SPACING` = 20ms) instead of each embedding its own `Duration::from_secs(2)` / `Duration::from_millis(20)` literal. If the removal itself already spent part of the 2-second budget retrying, the absence wait continues against the same deadline rather than opening a second independent window.

`wait_for_fixture_binary_absence` is deliberately not `#[cfg(windows)]`-gated, even though its only production caller is. The whole `native_tests` module is already `#[cfg(test)]`-only, so leaving the helper unconditional costs nothing in the production `--lib` build on any platform, and it lets the three new unit tests exercise the real function on macOS/Linux CI instead of a windows-only fixture nobody runs pre-merge — the same "never ran before it reached native CI" defect class this whole family of fixes exists to close. On unix a genuine removal is synchronous, so the first `symlink_metadata` check always observes `NotFound` and the function returns without ever sleeping.

## Risks / Trade-offs

The wait is a fixed poll against `symlink_metadata` rather than a native `CREATE_NEW` probe (which would additionally distinguish "gone" from "present but not delete-pending"). `symlink_metadata`'s `NotFound` is unambiguous evidence of absence on both platforms, so this is sufficient without adding a second Windows-only code path purely for the fixture. Exhaustion now reports path, attempt count and elapsed window once, on the terminal error path only, mirroring the message shape already established in `files.rs`'s `StageCleanupExhausted`.

## Operational surface

Windows-only native-CI fixture cleanup inside `provision::native_tests`, and the text of one fixture-exhaustion error. No bind address, secret, bundled asset, binary version, architecture or CI topology change.
