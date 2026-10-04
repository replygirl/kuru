# Design

## Context

`Directory::verify(name, file)` reads the held handle's identity, re-opens
`name` with `self.read(name)?` (which runs the regular-file, hardlink and privacy
checks on the current object first) and compares identities. A mismatch returned
`denied("file name no longer identifies the held object")`, a
`PermissionDenied` error indistinguishable from the privacy and identity
denials. `files::read_held_then` calls it after the held read and the `between`
hook. `EndpointRecord::read_then` maps only an absent name, or a failed read
whose probe finds the name absent, to `Ok(None)`; a successor owner publishing
over the name during handover therefore fails the elected starter's attach.

The held check was `checked_file(file)`, the strict inspection
(`regular_file_info` -> `native::info`). On Unix an object whose name a
publication replaced has `nlink == 0`, which `checked_file` reports as
`NotFound` "regular file was unlinked". On Windows, publication replaces the name
with `FILE_RENAME_FLAG_POSIX_SEMANTICS`, which leaves the held object
delete-pending with no link; the strict `inspect_info(file, false)` refuses any
delete-pending object as `PermissionDenied` "filesystem object is pending
deletion or has an invalid size". The platform already has a retained
inspection for exactly this state (`retained_file_info` ->
`native::retained_info` -> `inspect_info(file, true)`), used after checked
publication by `verify_retained_file_access` and `finalize_file_access`: it
accepts a delete-pending object only with zero links. On Unix
`retained_info` is `info` (`fs/unix.rs`), so the two inspections are identical
there.

## Goals / Non-Goals

**Goals:**
- A replaced endpoint record is a miss, so the existing readiness poll re-reads,
  on Windows as on Unix, wherever between the read and the verify the successor's
  publication lands.
- The replaced-name outcome is typed and distinct from every denial.

**Non-Goals:**
- No change to privacy checks, the poll's budget or the name-side check of
  `verify` (`self.read(name)?`, strict).
- No change to the admission check inside `Directory::read`'s own open, which
  stays strict.
- No retry loop, new wait or literal.
- No change to activity-record reads, which already decide nothing on a replaced
  name.

## Decisions

- Carry the outcome as a typed error source on the same `io::Error` that
  `verify` returns today, keeping `ErrorKind::PermissionDenied` and the message,
  so `?` propagation in existing callers is byte-identical; callers that care
  downcast to it. Rejected: a new return type for `verify`, which would change
  every caller, and matching the message text, which cannot be told from a
  denial structurally.
- The typed outcome is raised only after the re-opened name passed the regular
  file, hardlink and privacy checks, because `self.read(name)?` runs first. A
  replacement that is not a private regular file therefore still raises its own
  denial, not the typed outcome.
- `read_held_then` keeps its error type; the typed source survives
  `anyhow` wrapping so `read_then` can find it by downcast. Only `read_then` maps
  it to `Ok(None)`.
- `verify`'s held check inspects the held handle through `retained_file_info`
  and then applies the same one-link rule as `checked_file` (the shared
  `single_link` helper): zero links is `NotFound` "regular file was unlinked",
  more than one is denied. On Windows a held object that a POSIX replacement
  left delete-pending with no link now reports `NotFound`, as Unix already
  does, and `read_then`'s existing `NotFound` arm reads it as a miss with no new
  arm; a delete-pending object that still has a link stays refused by
  `retained_info`. Unix is unchanged. No new accessor is added to `fs/windows.rs`
  or `fs/unix.rs`, and no unsafe code is touched. Rejected: narrowing the
  documentation to the half of the window the typed outcome covers, which would
  leave the Windows attach failure in place.

## Accepted caller-visible change

Every `Directory::verify` caller on Windows now sees `NotFound` "regular file
was unlinked" (no OS code) instead of `PermissionDenied` "filesystem object is
pending deletion or has an invalid size" (no OS code) for a held file that is
delete-pending with no link. Unix callers already saw `NotFound` for the same
state. The lead accepted this change. Audit of every `.verify(` call site in
`packages` and `apps` (2026-10-04, by reading) for a branch on `NotFound`:

- Branch on `NotFound` and now behave on Windows as on Unix:
  - `kuru-memory` `EndpointRecord::read_then` (`service.rs`, through
    `files::read_bytes_then` -> `read_held_then`): `is_not_found` -> `Ok(None)`.
    The intended fix.
  - `kuru-memory` `server.rs` `read_record_then` (through `files::read_bytes`;
    reads `identity.json` and `endpoint.json`): `is_not_found` -> revalidate the
    parent, then `Ok(None)`. A record replaced while held now reads as absent on
    Windows as it already does on Unix, instead of failing.
  - `kuru-memory` `service.rs` `owner_timeline_clause` (test-support open
    timeline diagnostic, through `files::read_bytes`): a `NotFound` anywhere in
    the chain prints "owner timeline: absent" instead of "unreadable".
- Match `NotFound` but require an OS error code, so they classify the checked
  `NotFound` (no OS code) exactly as before: `files::is_missing_name`
  (`service/activity.rs` `inspect` and `unopened`) and `kuru-tui` `trust.rs`
  `ReadFailure::of`.
- Platform-internal callers keep their phase: `remove_file_then` maps a verify
  error to `Rejected`, `transfer_file_then` maps its preflight verify to
  `Rejected` and its post-move verify to `Uncertain`. Their consumers branch on
  the phase; the only downcasts of `PublicationError`/`RemovalError` outside
  the platform read directory moves, tree removals or a test's phase, never a
  file `verify` error's inner kind.
- `kuru-platform` `windows/process/image.rs` opens the current image with
  `FILE_SHARE_READ` only, so no rename can replace it while held.
- Every other caller (kuru-connectors, kuru-core, kuru-delivery, kuru-tui, the
  rest of kuru-memory) propagates with `?`, `.into()`, `map_err(|_| ..)`,
  `.is_err()` or `.is_ok()`; none reads the kind. `kuru-delivery`'s
  `bundle.rs` wrapper propagates, and `update.rs`, `archive.rs` and
  `shell_support.rs` test only `.is_err()`/`.is_ok()`.

## Risks / Trade-offs

- A reader mapping the outcome to a miss could hide a persistent replacement
  loop, which is bounded by the existing readiness budget and progress rule, so
  no new bound is added.
- The typed source must survive context wrapping, which a unit test in
  `kuru-memory` and one in `kuru-platform` pin.
- Residual (inference from code, not observed): a publication landing inside
  `Directory::read`'s own open, after the native open and before that open's
  strict `checked_file`, still reports `PermissionDenied` "pending deletion" on
  Windows where Unix reports `NotFound`. In `read_then` that error is neither
  `NotFound` nor the typed outcome, and the probe finds the successor's record,
  so that attach still fails on Windows. The window is not a few instructions:
  `open_file`'s admission (`checked_file` -> `regular_file_info` ->
  `inspect_info`) reads the delete-pending bit in `inspect_info`'s second
  metadata query (`FileStandardInfo`, after the `FileAttributeTagInfo` query),
  so it spans `CreateFileW` returning through two metadata syscalls. That is the
  same order as the read-to-verify half this change closes (a `ReadFile`, the
  `between` hook and one or two metadata syscalls), so it must not be read as
  negligible. It is ordinary admission, which this change keeps strict by
  decision; the documentation claims only the window between a read and its
  verify. If it is ever observed (that error text with
  `read private memory service endpoint` on Windows), the open's held-handle
  inspection is the place to decide.
  - Fix shape: the same `single_link(retained_file_info(..))` swap at
    `open_file`'s admission (about `fs.rs:590`). It admits no object that strict
    admission refuses: a delete-pending object with zero links becomes
    `NotFound` instead of `PermissionDenied`, and one with a live link stays
    denied. It does change the kind every checked open's caller sees on Windows
    for that state (`Directory::read`, `read_write`, `create_new`, `lock_file`),
    and about 25 of those callers branch on `NotFound` (`update.rs`
    `absent`/`load`, `file_edits.rs` `target_snapshot`, `auth/store.rs`
    `read_record`, `permission_store.rs`, `trust.rs`, `memory_export.rs`,
    `mcp_cache.rs`, among others). That wider caller audit is why it needs its
    own change rather than this one.
  - Scheduling: a scheduled follow-on, not left to lapse. It was catalogued in
    the team's flake catalogue on 2026-10-04 and reported to the lead for
    assignment.
- On Windows the retirement path deletes its staged record with the legacy
  delete disposition, which keeps the name occupied while a reader holds it;
  `retained_info` keeps refusing that linked delete-pending object, so
  `a_read_that_meets_retirement_finds_no_record` still reaches `read_then`'s
  probe arm (inference: it assumes NTFS reports the occupied name as a link;
  the test asserts only `Ok(None)`, which holds either way). The judgement that
  this case is unreachable in practice is inference from the documented close
  ordering (`docs/development.md`: retire inside close) and the maintenance
  loop's lock gate, not traced in code: a successor can publish only after
  acquiring the owner lock that the retiring owner releases after its store
  close, which is seconds against the reader's microsecond read-to-verify gap.
  The owner-lock release site was not read.
