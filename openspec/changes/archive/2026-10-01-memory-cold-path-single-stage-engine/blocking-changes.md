# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Notes

- The stage worker this change edits
  (`packages/kuru-memory/src/store/stage_worker.rs`) and its single-engine
  `build_template` job already landed as the archived
  `2026-09-30-memory-stage-worker-extraction`,
  `2026-09-30-memory-template-cache`, `2026-09-30-memory-template-stage-adoption`,
  `2026-10-01-memory-template-creation` and
  `2026-10-01-memory-template-post-build-copy` changes. Those already shipped,
  so they are prerequisites satisfied by history, not an open blocker.
- `openspec/changes/` holds no other active change at the time this change
  was scaffolded, so there is nothing else to sequence against.
- Parallel, non-blocking work: a sibling branch develops P6 (migration
  records, schema 8) touching `store/migrations.rs`. This change does not
  touch the migration registry and keeps its edits to the stage worker,
  `open_inner`'s cold-path call and tests, so that branch's rebase onto this
  one stays mechanical; neither change blocks the other.
