# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

Not gating; recorded so the later rebase is deliberate. Neither sibling is an active change in this tree (checked 2026-09-29: `openspec/changes/` holds only this change and `archive/`); the list was not confirmed with the maintainer from this authoring stage.

- `memory-definite-reconcile-answers` (branch `fix/memory-definite-reconcile-answers`, unit 3) edits `ReceiptProgress` and the outcome handlers in `packages/kuru-memory/src/service/rpc.rs`. This change keeps its `rpc.rs` edits to `Retirement`, `serve_attached` and the handshake. Unit 3 also removes `ReplyPause::replied`, `ReplyBarrier::wait_replied` and the related `replied` waits; this change's T8 is written against today's `replied` notify, so whichever merges second rewrites T8's synchronisation on the surviving hook. Unit 3's owner-side client-gone release (a waiting outcome query ends when its client disconnects) interacts with the held stream of Option B only in that the held stream is never read or written by the client again; no ordering dependency.
- Unit 2 (open-activity feedback, not yet a change) is stacked on this change and consumes the starter token introduced here (`OpenOptions::starter_token`, the optional tenth service argument, the hello field). The owner writes unit 2's activity record on every start.
