# Dependencies

## Blocked by

- [x] `atomic-compact-checkpoint-provenance` — supplies the forward-compatible operation provenance and one atomic context-summary plus producer-private reasoning checkpoint required for accepted internal Compact results; P09 must not fabricate a conversation turn or split the summary and sidecars across receipts. *(archived 2026-09-22)*
- [x] `session-cursor-history-window` — committed P29 head `49b765e` supplies session-filtered sequenced source snapshots, atomic summary-plus-cursor publication and the cursor-selected summary projection. P09 additionally requires the P29-owned bounded newest session-history suffix strictly after an exact cursor, retaining sequences/view/revision and eligible-row count across local/remote main/candidate views; the existing newest unsequenced window and oldest forward page cannot compose this for more than 1,024 eligible rows. *(archived 2026-09-22)*

## Soft-blocked by

None.

## Phase gates

The reviewed P29 follow-up is limited to a read-only cursor-aware newest-suffix projection; P09 must not page unboundedly, infer sequences, or read SQL/export rows around it. Provider reasoning summaries, including operation-attributed Compact sidecars, remain distinct producer-private records and are never imported as cross-session continuity.
