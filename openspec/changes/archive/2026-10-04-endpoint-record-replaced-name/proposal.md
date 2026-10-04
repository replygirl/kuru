# Proposal

## Why

An elected starter's readiness attach can fail hard while a successor owner
publishes during handover. The starter reads the private memory service
endpoint record through a held handle, and `Directory::verify` then re-opens the
record's name to confirm it still identifies that handle. When the successor
has published a new record over the name in between, the identities differ and
the read fails with `file name no longer identifies the held object`, which
`EndpointRecord::read_then` wraps as `read private memory service endpoint`.
The readiness design already treats a changed record as a reason to keep
waiting, but this error ends the attach instead of the next poll re-reading.

Measured: PR #205 CI run 37172801478, macos-latest coverage partition 1, job
111349259448, failed `service::tests::crashed_owner_retains_accepted_receipt_after_sibling_write`
with `successor election and readiness after crashed endpoint retirement`,
`attach after starting the elected memory service; child remained running`,
`read private memory service endpoint`, `file name no longer identifies the
held object`. The same test and error occurred locally on macOS on 2026-10-02.
The cause is not recoverable by the caller today because `Directory::verify`
reports a replaced name as `io::ErrorKind::PermissionDenied`, the same kind as
the privacy and identity denials, so a reader cannot tell a replaced name from a
privacy defect.

Independent review of PR #209 (head 124012e7) found that on Windows, for a
successor publishing directly over the held record, the typed outcome covers
only a publication landing inside `verify`, between its held check and its
reopen of the name. A publication landing earlier, after the
reader's byte read and before `verify`'s held check, leaves the held object
delete-pending with no link; `verify`'s strict held check refused that as an
untyped `PermissionDenied` ("pending deletion"), where Unix reports the same
state as `NotFound` ("regular file was unlinked"), which `read_then` already
reads as a miss. The attach could therefore still fail on Windows (inferred
from code; not observed natively).

## What Changes

- `Directory::verify` reports a replaced name as a typed outcome distinct from
  every denial. Existing callers that propagate the error with `?` observe the
  same kind and message as before.
- `Directory::verify` inspects its held handle through the platform's retained
  inspection (`retained_file_info`) instead of the strict one. On Windows a POSIX
  replacement of the name leaves the held object delete-pending with no link;
  the strict inspection refused that as an untyped `PermissionDenied` ("pending
  deletion"), so a successor publishing between the reader's byte read and
  `verify`'s held check still failed the attach. It now reports `NotFound`
  "regular file was unlinked", exactly as Unix already does for the same state.
  A delete-pending object that still has a link stays refused. Unix behaviour is
  unchanged (its retained inspection is its strict one).
- `files::read_held_then` is unchanged: its `?` keeps the `io::Error`, typed
  source included, at the root of the `anyhow::Error`.
- `EndpointRecord::read_then` maps the typed outcome to `Ok(None)`, a miss, and
  its existing `NotFound` arm now covers the unlinked held record on Windows as
  on Unix, so the existing readiness poll re-reads under its existing budget and
  the next read returns the successor's record. A replacement that is not a
  private regular file, any privacy denial and every other error keep failing as
  today.
- The documentation states that a record replaced between a read and its verify
  is a miss on both platforms (`docs/memory.md`), and the activity-record
  sentence in `docs/development.md` now says the held-handle check reports an
  unlinked record on Windows as on Unix.

No retry loop, wait, literal, budget or privacy check is added or changed.

One caller-visible change is accepted by the lead: every other
`Directory::verify` caller on Windows now sees `NotFound` "regular file was
unlinked" instead of `PermissionDenied` "filesystem object is pending deletion
or has an invalid size" for a held file left delete-pending with no link, the
state a POSIX replacement of its name produces, matching what Unix callers
already see for an unlinked held file. The callers that branch on `NotFound`
are listed in `design.md`.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living `project-memory-owner` requirement "Readiness wait bounded by
owner progress" and `project-memory-service` spec were right; they do not state
this reader's behaviour, and the implementation contradicted the readiness
design. No delta spec is needed.

## Impact

- `packages/kuru-platform/src/fs.rs` (`Directory::verify`: replaced-name outcome
  and retained held-handle inspection; unit tests)
- `packages/kuru-memory/src/service.rs` (`EndpointRecord::read_then` and tests)
- `packages/kuru-memory/src/files.rs` unchanged; `fs/windows.rs` and
  `fs/unix.rs` unchanged (the existing `retained_file_info` is reused).
- `docs/memory.md` beside the retiring-service wait paragraph, and the
  activity-record sentence in `docs/development.md`.
- No dependency, protocol or configuration change. The public API change is
  additive: `kuru_platform::fs::is_name_replaced`. Other `verify` callers keep
  their kind and message, except the accepted Windows `NotFound` parity above.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
