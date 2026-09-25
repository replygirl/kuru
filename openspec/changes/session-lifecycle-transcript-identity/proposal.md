## Why

Kuru can resume an opaque session ID, but its session index and transcript rows do not provide a durable lifecycle, a settled-turn boundary, or the historical speaker identity needed to rename, remove, restore, fork, and export one conversation safely. P29 now supplies session-scoped private memory and pinned managed-store operations, so P11 can add the public conversation contract once without confusing chat history with shared project memory or pre-empting P30's concurrent-driver admission.

## What Changes

- Replace the opaque session-list blob with a versioned session catalog and durable turn-indexed public transcript records carrying exact session, turn, settlement, stable speaker, typed content, and fork provenance.
- Add rename and reversible removal/restore. Removal excludes a session from ordinary listings, continue, picker, and resume while retaining its transcript, project memory, audit history, and an explicit restoration path.
- Add fork through an exact completed or durably interrupted turn. A fork preserves the parent, exposes an independent public transcript prefix, records its source, refuses a pending boundary, and clearly states that current project memory remains shared rather than historically branched.
- Add bounded, revision-checked session inspection and transcript paging for local and managed live views. Mutations use typed receipt-backed outcomes so accepted lost replies reconcile without replay or false success.
- Add one-session Markdown and JSONL export with stable chronological ordering, settlement and speaker provenance, typed content, fork disclosure, and no private actor history, reasoning sidecar, summary, note, candidate, or unrelated-session rows.
- Add `--continue`, CLI session lifecycle commands, and the shared-registry `/new`, `/sessions`, `/resume`, and `/export` handlers with a real TUI picker. A new invocation resets session-only grants. P11 retains the current single conversation-driver admission; P30 later adds live-session leases and default concurrent new-session startup.
- Migrate only transcript/session associations supported by durable evidence. A validated legacy transcript namespace and saved session may establish the session and order while absent speaker evidence renders as `unknown`; rows that cannot be assigned remain available through the existing full-memory inspection/export rather than disappearing or being injected into every session. No current actor is guessed from prompt text or present topology.

## Capabilities

### New Capabilities

- `session-lifecycle`: Durable session catalog, settled-turn public transcript identity, reversible lifecycle, bounded fork and export, continue semantics, and CLI/TUI session management.

### Modified Capabilities

- `chat-harness`: Strengthen durable session continuity with exact continue/resume rules, settled public transcript projection, legacy identity handling, and the boundary between P11 lifecycle and P30 live-driver admission.
- `command-registry`: Register and route the working session commands through the existing shared catalog only when their P11 backends are present.

## Impact

- `packages/kuru-memory/src/store.rs`, `facade.rs`, `service.rs`, `service/rpc.rs`, `store/migrations.rs`, `store/export.rs`, and focused fixtures gain the v7 session/turn schema, typed local/remote reads and receipt-bearing lifecycle mutations.
- `packages/kuru-runtime/src/engine.rs` and focused runtime modules/tests use the durable catalog and settled-turn boundary for create, resume, continue, interruption, fork, and transcript projection without copying raw private memory.
- `apps/kuru-tui/src/cli.rs`, `commands.rs`, `ui.rs`, rendering/tests, and `docs/usage.md` gain the lifecycle commands, picker, export, disclosures, and legacy labels.
- The change depends on the P27 managed memory service and its P28 receipts, and on P29 session provenance plus the bounded cursor history seam. It coordinates with P08/P09 privacy and summary provenance, but consumes only public transcript records and does not wait for private-summary presentation work.
- P30 consumes this catalog and its active/inactive-independent session identity later; P11 does not relax the current project/conversation driver lease or add second-surface attachment.
- The store schema advances from v6 to v7 after the archived atomic Compact provenance migration. The managed protocol gains typed session reads and mutations. Existing session IDs, v6 context summaries and private reasoning rows remain valid, legacy data remains retained, and no public transcript row is silently assigned a present-day speaker.

## Surfaces

- [x] interactive — CLI, slash commands, picker, transcript replay, export, and lifecycle diagnostics are user-visible.
- [x] deploy — the managed memory owner protocol and store migration run in ordinary local deployments.
- [ ] integration — no third-party service or protocol is added.
- [ ] agent-behavior — no prompt, model routing, tool authority, or agent output contract changes.
