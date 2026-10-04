# Design

## Context

`Directory::verify(name, file)` reads the held handle's identity, re-opens
`name` with `self.read(name)?` (which runs the regular-file, hardlink and privacy
checks on the current object first) and compares identities. A mismatch returns
`denied("file name no longer identifies the held object")`, a
`PermissionDenied` error indistinguishable from the privacy and identity
denials. `files::read_held_then` calls it after the held read and the `between`
hook. `EndpointRecord::read_then` maps only an absent name, or a failed read
whose probe finds the name absent, to `Ok(None)`; a successor owner publishing
over the name during handover therefore fails the elected starter's attach.

## Goals / Non-Goals

**Goals:**
- A replaced endpoint record is a miss, so the existing readiness poll re-reads.
- The replaced-name outcome is typed and distinct from every denial.

**Non-Goals:**
- No change to privacy checks, the poll's budget or other `verify` callers.
- No retry loop, new wait or literal.
- No change to activity-record reads, which already decide nothing on a replaced name.

## Decisions

- Carry the outcome as a typed error source on the same `io::Error` that
  `verify` returns today, keeping `ErrorKind::PermissionDenied` and the message,
  so `?` propagation in existing callers is byte-identical; callers that care
  downcast to it. Rejected: a new return type for `verify`, which would change
  every caller, and matching the message text, which cannot be told from a
  denial structurally.
- The typed outcome is raised only after the re-opened name passed the regular
  file, hardlink and privacy checks, because `self.read(name)?` runs first. A
  non-private replacement therefore still raises its own denial, not the typed
  outcome.
- `read_held_then` keeps its error type; the typed source survives
  `anyhow` wrapping so `read_then` can find it by downcast. Only `read_then` maps
  it to `Ok(None)`.

## Risks / Trade-offs

- A reader mapping the outcome to a miss could hide a persistent replacement
  loop, which is bounded by the existing readiness budget and progress rule, so
  no new bound is added.
- The typed source must survive context wrapping, which a unit test in
  `kuru-memory` and one in `kuru-platform` pin.
