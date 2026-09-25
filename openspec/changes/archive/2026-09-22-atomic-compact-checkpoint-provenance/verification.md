## 1. Forward migration and exact legacy identity [critical]

- [x] 1.1 @integration (agent) upgrade a real v5 store containing turn-attributed context and private reasoning rows to v6, reopen and export/restore it -> `store::migrations::tests::released_v5_summary_identity_survives_v6_upgrade_reopen_and_export` passed 1/1 in 4.26s against real Dolt; it retained the exact summary ID, v1 format, state key/value, turn-only provenance and v4 usage branch across migration, active export and reopen.
- [x] 1.2 @unit (agent) derive IDs, keys and serialized values for legacy and operation records -> `store::tests::legacy_summary_serialization_and_identities_remain_byte_exact` passed 1/1; exact pre-v6 JSON, key and context-ID derivations remained byte-identical while operation records used separate domains and format.

## 2. Atomic checkpoint and typed no-effect failures [critical]

- [x] 2.1 @integration (agent) checkpoint a context summary with multiple private sidecar records against real Dolt, then reopen -> `store::tests::compact_checkpoint_sidecars_are_atomic_on_stale_and_late_conflict` passed 1/1 in 4.45s; both sidecars and the exact operation/producer-attributed summary survived close/reopen with one current cursor selection.
- [x] 2.2 @integration (agent) submit stale revision/cursor and a conflict on a later sidecar record -> the same focused real-Dolt fixture observed typed `ContextSummaryStale` and `ReasoningSummaryConflict`; the fresh earlier sidecar, summary and cursor remained absent after each refusal before the successful retry.

## 3. Managed bounds and lost-reply reconciliation [critical]

- [x] 3.1 @integration (agent) reject invalid binding, worst-escape exact-limit and one-byte-over combined envelopes through local and remote facades -> `service::rpc::tests::compact_checkpoint_enforces_exact_serialized_bound_below_rpc_frame` passed 1/1 in 3.35s at exactly 64 MiB and rejected +1; the managed facade fixture rejected mismatched provenance and an over-limit escaped payload without sending the armed RPC frame, while the successful owner path repeated the shared validator.
- [x] 3.2 @integration (agent) drop an accepted managed checkpoint reply, query its logical outcome and retry exactly -> `facade::tests::remote_session_checkpoint_lost_reply_preserves_pinned_provenance` passed 1/1 in 4.17s after the final bound strengthening; the lost reply fenced mutation, indexed reconciliation proved the whole summary/cursor/sidecar commit, the same UUID and payload returned the retained receipt, a fresh UUID was typed stale, and no duplicate or prefix appeared.

## 4. Repository gates

- [x] 4.1 @regression (agent) run focused store/facade/service fixtures, all-target memory typecheck and strict Cospec validation -> the five focused fixtures above passed; `cargo check -p kuru-memory --all-targets --features test-support` passed in 3.44s with the verified offline bundle mirror; `cargo fmt --all -- --check` and `git diff --check` passed. The first real-Dolt launch was sandbox-blocked while reserving a private loopback port and is not product evidence; its authorized rerun passed. Strict Cospec validation and final committed-head verification are recorded during archive preparation.

The initial pure-test command used `--exact` without the module-qualified test names and ran zero tests; it is not evidence. The corrected filters above each ran one named test. Full workspace coverage and hosted native CI are outside this bounded prerequisite and remain pending in their owning delivery gates.

The exact storage API prerequisite is commit `b398c31` atop artifact/gate commit `0e90b6a` and cursor-window base `0d4895e`. It exports `ContextSummaryCheckpoint`, advances only the main schema to v6, and changes `MemoryStore::checkpoint_context_summary` to accept the complete typed envelope; actor-context-compaction can consume that commit directly and remove its exact `atomic-compact-checkpoint-provenance` blocker without copying the migration, facade or RPC files.
