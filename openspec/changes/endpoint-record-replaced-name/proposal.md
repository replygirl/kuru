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

## What Changes

- `Directory::verify` reports a replaced name as a typed outcome distinct from
  every denial. Existing callers that propagate the error with `?` observe the
  same kind and message as before.
- The typed outcome is carried through `files::read_held_then`.
- `EndpointRecord::read_then` maps only that outcome to `Ok(None)`, a miss, so
  the existing readiness poll re-reads under its existing budget and the next
  read returns the successor's record. A replacement that is not private, any
  privacy denial and every other error keep failing as today.
- One sentence in the documentation states that a record replaced during a read
  is a miss that the next poll re-reads.

No retry loop, wait, literal, budget or privacy check is added or changed, and
no other `verify` caller changes behaviour.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living `project-memory-owner` requirement "Readiness wait bounded by
owner progress" and `project-memory-service` spec were right; they do not state
this reader's behaviour, and the implementation contradicted the readiness
design. No delta spec is needed.

## Impact

- `packages/kuru-platform/src/fs.rs` (`Directory::verify`, replaced-name outcome
  and unit test)
- `packages/kuru-memory/src/files.rs` (`read_held_then`)
- `packages/kuru-memory/src/service.rs` (`EndpointRecord::read_then` and tests)
- One sentence in `docs/memory.md` beside the retiring-service wait paragraph.
- No dependency, protocol, configuration or public API change; other `verify`
  callers in kuru-connectors and kuru-memory are unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
