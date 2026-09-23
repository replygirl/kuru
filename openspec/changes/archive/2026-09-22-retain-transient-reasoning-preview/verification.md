## 1. Selected-speaker transient preview [critical]

- [x] 1.1 @regression (agent) stream a reasoning-summary delta through the selected speaker's active progress observer -> the existing bounded text tail is visible, then clears with the turn; no provider coordinates are exposed (observed 2026-09-22: runtime progress boundary 1/1; selected-speaker and tool-loop regressions 2/2).
- [~] 1.2 @e2e (agent) render the active preview at practical terminal sizes -> defer: this P08 lineage preserves P27's candidate UI dispatch but awaits its final command-registry union with P04; the attempted exact renderer test stopped at that known compile boundary before execution.

## 2. Settled record boundary

- [x] 2.1 @integration (agent) emit a settled reasoning-summary sidecar after an active preview -> progress does not copy the settled record or its coordinates into durable public event detail (observed 2026-09-22: runtime progress boundary 1/1).
