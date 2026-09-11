# Dependencies

## Blocked by

- [x] `native-windows` — supplies the five-target application, private filesystem and owned process integration, native terminal/authentication lifecycle, Windows distribution and update contracts this client bundle must preserve. *(archived 2026-09-11)*

## Soft-blocked by

None.

## Sequencing and evidence

The user's portable, complete-install requirement and the explicitly assigned
follow-up sequence establish this hard dependency. Every active proposal was
reviewed: `native-platform` supplies independent native primitives,
`embedded-runtime` supplies Dolt embedding and local build preparation, and
`native-windows` consumes both before this provider-client integration begins.
Those transitive prerequisites remain mandatory; this change does not reverse
their dependency direction or authorize a Windows product implementation early.

Archived `peer-harness`, `native-delivery`, `direct-release-install`, release
recovery and release-only documentation changes establish the existing provider
transport, Rust tooling, compiler-free installation and publication contracts.
They are baseline behavior rather than unresolved blockers. No source change may
begin while the actual cospec apply result remains blocked.
