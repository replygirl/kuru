# Dependencies

## Blocked by

<!-- Order 1's prerequisites from the per-machine template design's PR split
     (tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md section
     11, "P4a | ... | P1, P2") are already archived on this tree. -->

- [x] `memory-engine-contract-tests` — P1 engine spike (S1-S8), measured against real Dolt 2.3.5, that this change's adoption group relies on *(archived 2026-09-29)*
- [x] `main-pool-migration-classification` — P2 main-pool classification of retained migration branches, that this change's template-shape check and recovery classes rely on *(archived 2026-09-30)*
- [x] `memory-stage-worker-extraction` — P3 stage worker extraction, that this change's adoption bootstrap runs inside *(archived 2026-09-30)*

## Soft-blocked by

None.

<!-- P4b (template cache, key computation, copy worker, creation selector)
     depends on this change, not the reverse: this change has no caller yet,
     so no open PR degrades without it landing first. -->
