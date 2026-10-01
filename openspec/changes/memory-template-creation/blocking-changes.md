# Dependencies

## Blocked by

None. The per-machine store template cache and the template-born stage
identity/adoption protocol this change wires into an ordinary open are
already on `main` (the `versioned-memory` requirements "Per-machine store
template cache" and "Template-born stage identity and adoption", and the
`creation_template`, `stage_worker` and adoption-bootstrap code they
describe), not a pending change in `openspec/changes/`.

Landed prerequisites this change builds on, all merged to `main` before this
change's artifacts were authored: `2026-09-30-memory-stage-worker-extraction`
and `2026-09-30-memory-template-cache` (#145, #147: `creation_template`,
`stage_worker`, the adoption bootstrap), and `2026-09-30-memory-open-activity`
(#148: the owner-published activity record and the five `memory_activity.rs`
sentences this change's own creation path now runs under). None of these are
pending changes; they are cited here only so a reader of this change's design
can find the decisions it depends on.

## Soft-blocked by

None.
