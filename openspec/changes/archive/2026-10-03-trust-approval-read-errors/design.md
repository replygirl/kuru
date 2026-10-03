# Design

## Context

`ApprovalStore::read_record` returns `Err` for several unlike causes: OS read
failures (`EACCES`, `EIO`) from opening or reading the record or its storage
directories, and rejections by `kuru-platform`'s checked filesystem (a
non-private mode, a hard link, a directory where a regular file belongs, a
symlink, a replaced object). The checked filesystem synthesizes its own
rejections with `io::Error::new`, mostly as `ErrorKind::PermissionDenied`, so
the error kind alone cannot tell "permission denied by the OS" from "rejected
as unsafe". The `Err(_)` arms of `inspect` (trust.rs:231) and `inspect_nested`
(trust.rs:279) and the `unwrap_or(false)` in the instruction gate
(instruction_gate.rs:231) discarded that difference.

## Goals / Non-Goals

**Goals:**
- Name a genuine OS read failure, and give a remedy that can work.
- Keep every existing structural rejection reported as invalid or unsafe.
- Keep every non-approving outcome non-approving.

**Non-Goals:**
- Changing what any state approves, the record format or the store layout.
- Retrying reads, or changing the preflight's general remedy sentence.
- Classifying `approve`, `approve_nested` or `revoke` errors (they already
  surface their own errors).

## Decisions

- D1. A read failure is an error whose first `io::Error` in the chain carries a
  raw OS code (`raw_os_error()`); synthesized platform rejections have none.
  Rejected alternative: matching `ErrorKind::PermissionDenied`, which would
  relabel the platform's synthesized not-private and hard-link rejections.
- D2. OS codes that describe object shape stay `Invalid`: `NotFound`,
  `NotADirectory`, `IsADirectory`, and on Unix `ELOOP` (the checked open uses
  `O_NOFOLLOW`, so a symlink at the record path fails with a raw `ELOOP`;
  the existing `symlinked_and_permissive_approval_objects_never_match` test
  asserts `Invalid`). Windows rejects reparse points with a synthesized error.
- D3. `Unreadable(ReadFailure)` carries an `i32` code, not a `String`, keeping
  `ApprovalState: Copy + Eq` for its `==` comparisons, and displays through
  `io::Error::from_raw_os_error`, which yields the platform's standard message
  and no path. `EIO` keeps its text even though its `ErrorKind` is unnameable.
- D4. `inspect_nested`'s `Unreadable` returns no review generation, so a
  persistent nested choice is refused exactly as for `Invalid`.
- D5. `publish()` propagates `generation_is_current` errors with the context
  "workspace approval record could not be read". This fails closed as before;
  it may also carry a structural rejection under that context, whose chain
  names the actual cause.
- D6. The `invalid source set` return at trust.rs:243 is a caller-supplied
  shape check made before any read, so it stays `Invalid`.

## Risks / Trade-offs

- [On Windows, opening a directory without backup semantics fails with a raw
  `ERROR_ACCESS_DENIED`, so a directory in the record's place reads as
  `Unreadable` there, not `Invalid`] → it is still non-approving and the remedy
  (inspect the record's permissions) leads to the object; the cross-platform
  test asserts only that it never approves, and asserts `Invalid` on Unix.
- [A future platform rejection that returns a raw OS code would be labelled
  unreadable] → both states fail closed; the label is diagnostic only.

## Operational surface

The change is local to the `kuru` binary's CLI and TUI text: `trust status`,
the preflight refusal and the nested instruction review show the new status
line. No bind address, container, runner topology, secret, connection limit,
binary version or architecture changes. The `ELOOP` exclusion is compiled only
on Unix; Windows builds compile the same classifier without it and are checked
by `lint:windows`.
