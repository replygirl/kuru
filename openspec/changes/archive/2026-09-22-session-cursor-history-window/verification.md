## 1. Exact newest suffix after a cursor [critical]

- [x] 1.1 @integration (agent) seed one real-Dolt session with 1,025 post-cursor rows plus interleaved sibling-session, sibling-namespace and unattributed rows -> `store::tests::session_source_snapshot_and_context_checkpoint_preserve_exact_provenance` passed 1/1 in 3.10s; it reported 1,025 eligible rows and returned only the newest 16 exact target-session sequences in ascending order
- [x] 1.2 @integration (agent) read the same cursor-relative suffix through managed main and candidate views after candidate-only and live-only appends -> the local fixture above and `facade::tests::remote_session_checkpoint_lost_reply_preserves_pinned_provenance` passed 1/1 in 2.54s with exact main/candidate view provenance and no unpromoted row crossing views

## 2. Bounds and compatibility [critical]

- [x] 2.1 @regression (agent) exercise zero limit, exact-latest cursor, invalid cursor/limit and aggregate serialized-byte exhaustion -> the local real-Dolt fixture passed its count-only, empty and invalid-bound assertions; `store::tests::session_source_budget_rejects_the_next_row_or_byte_without_partial_accounting` passed 1/1 and preserved accounting at the 32 MiB refusal
- [x] 2.2 @integration (agent) run all-target memory typecheck plus the managed fixture against the advanced protocol minor -> `//packages/kuru-memory:typecheck` passed all targets/features in 12.01s, the managed fixture passed, and `service::rpc::tests::session_cursor_history_is_read_only_and_has_no_unit_receipt` passed 1/1

Observed setup notes, 2026-09-22: package-owned prefetch succeeded with the verified read-only bundle mirror. The first local command used a short filter with `--exact` and selected zero tests; the corrected full module path produced the 1/1 result above. The first byte-budget unit command omitted the bundle mirror environment and stopped in the build script before testing; the corrected invocation produced the 1/1 result above. ConfigSol independently source-cleared the final transaction, DTO, facade/RPC, protocol and fixture boundaries. No combined coverage or hosted native result is claimed for this narrow dependent change.
