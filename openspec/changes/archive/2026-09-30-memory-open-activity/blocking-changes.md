# Dependencies

## Blocked by

- [x] `memory-owner-immediate-retirement` — unit 1: the starter token (`OpenOptions::starter_token`, the optional tenth service argument, the hello field) and the owner close order this change extends *(archived 2026-09-30)*
- [x] `memory-stage-worker-extraction` — the `StageWorker` and `Creation` selector in `open_inner` that the new stage sites are placed against *(archived 2026-09-30)*
- [x] `drop-warm-engine-version-probe` — the current `versioned-memory` requirement text these deltas modify *(archived 2026-09-30)*

## Soft-blocked by

None.

## Siblings

Not gating; recorded so the later rebase is deliberate.

- Unit 1 (`memory-owner-immediate-retirement`): this change neither changes the starter token's argument position nor its parse rule (merged rule: `Uuid::parse_str` only, no version check, an 11th argument rejected), adds no second token and no `open_started`. It reads `OpenOptions::starter_token` inside the owner's open. The record stores a SHA-256 derived tag, never the raw admission token.
- The template-copy change for new projects (`feat-memory-template-adoption`, branch `feat/memory-template-adoption`): it rewrites the `open_inner` creation region. `CreatingDatabase` stays the first statement in whatever branch handles a project with no active store, and `UpgradingDatabase` stays first in the writable upgrade block.
- The open-time harness (PR #133, branch `ci/open-time-report`, owner assistant3): not merged at `cb1c8e1d`. This change does not wait for it; the harness accepts either the old `Memory: ready.` line or the marker lines, and is told this change's PR number when it opens. If #133 merges first its gate must already accept the markers.
- `feat-explicit-legacy-memory-import`: may touch the legacy-import branch of `open_inner`; `CreatingDatabase` precedes the import whatever it becomes.
