# Dependencies

## Blocked by

<!-- Order 1's prerequisites from the per-machine template design's PR split
     (tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md section
     11, "P4b-i | ... | P3, P4a") are already archived on this tree. -->

- [x] `memory-engine-contract-tests` — P1 engine spike (S1-S8), measured against real Dolt 2.3.5, that the build's `validate_active`/shape assertions and the byte-scan guard rely on *(archived 2026-09-29)*
- [x] `main-pool-migration-classification` — P2 main-pool classification of retained migration branches, that the template-shape check's branch classification relies on *(archived 2026-09-30)*
- [x] `memory-stage-worker-extraction` — P3 stage worker extraction (the `StageWorker` job shape this change's `TemplateBuild` job is added to) *(archived 2026-09-30)*
- [x] `memory-template-stage-adoption` — P4a adoption and recovery (the identity marker, the typed `TemplateRejected` verdict, the shared `store/migrations/template_shape.rs` check, and recovery classes R and U) that this change's build, publication and copy-verification primitives are built to feed *(archived 2026-09-30, #145)*

## Soft-blocked by

None.

<!-- P4b-ii (routing Creation::Default through the template, the copy worker,
     cold fallbacks) depends on this change, not the reverse: this change has
     no caller yet, so no open PR degrades without it landing first. -->

## Live worktrees checked for conflict on `store.rs` / provisioning

Per the design's section 11 note to order this change against other live
worktrees touching the same files: `gh pr list --repo replygirl/kuru --state
open` shows only #139 (dependabot cargo bump) and #133 (`ci/open-time-report`,
draft, CI-only, owned by another agent per the design's P0 row) open. The four
worktrees the design named
(`perf-memory-drop-warm-version-probe`, `feat-first-launch-feedback`,
`phase2-warm-service-reuse`, `fix-memory-service-retire-when-unused`) have no
corresponding open PR; `drop-warm-engine-version-probe` and
`memory-owner-immediate-retirement` are already archived on this tree's main.
Neither open PR touches `store.rs`, `provision.rs`, `stage_worker.rs`,
`test_support.rs` or `test_support/template.rs`, so none is declared as a
blocker here.
