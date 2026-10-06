# Design

## Context

`cli.rs` retains one project OS writer lock for all ordinary writers. The memory owner already serializes short mutation workers through `Shared.write`; `serve_attached` drops connection resources on EOF, and reached owners retire immediately when their attachment tasks finish. `RemoteSession` has primary and ephemeral SQL-exchange attachments, cancelled primary calls retain their abandoned streams during replacement, and candidate/read-cut/dream resources already have separate attachment lifetimes. Neither the current handshake nor the store carries driver identity.

`Harness::resume_session` prepares configuration, profile and topology, then rereads the catalog before publishing local state. That read is currently separate from ownership. `new_session` currently assigns local state before its fallible save. The existing CancellationToken reaches actor/provider/tool futures, but memory ownership loss does not signal it. Native shell workers retain cleanup outside a dropped future and `ToolHost::shutdown` confirms their local teardown. An already accepted remote request cannot be undone by EOF or cancellation.

## Goals / Non-Goals

Implement the proposal and its checked selection/private-history requirements with the existing concrete store, mutation guard, native private-file primitives and cancellation/cleanup paths. No durable live-registration schema, arbitrary SQL, control API, general lease scheduler, timer reclamation, private lineage, provider daemon, new process primitive or attachment-cap adjustment.

## Decisions

### One owner claim table, one dedicated presence attachment

Add a small session-claims child module on the owning store's Shared state. Claim entries bind session, client UUID, claim UUID, owner generation, catalog expectation and monotonic creation observation. The same owner write guard serializes claim transitions, session checkpoints/appends and catalog lifecycle mutations. A connection-held claim guard removes only its exact claim identity on drop; accepted mutation workers retain the write guard until their own settlement, so a successor claim cannot overtake an accepted private write.

Add an optional client UUID to ClientHello for decoding older peers, require it for the new compatible writable protocol, and propagate the same UUID through one RemoteSession's primary, extra, candidate and lease attachments. This identity selects an authenticated client's ownership; it is not a PID, credential, arbitrary ref or new endpoint authority. Driver-bound local views mint per-driver identity rather than treating cloned Local stores as one driver. Existing non-driving inspection/shared-memory operations do not implicitly gain a driver claim.

Each Harness receives a distinct driver-bound facade view, even when two constructors are passed clones of one RemoteSession. Resolve the input's existing mutation fence before establishing that bound view; creating a new identity must not bypass an unresolved write. Candidate views preserve their initiating driver's identity and exact proof. Claim proof remains stable across permitted own rename or catalog checkpoints; the full captured catalog expectation is checked separately at a later selection boundary.

A dedicated connection owns the driver guard and supplies lifetime observation. It is not a primary SQL exchange stream and is never automatically replaced as an ordinary abandoned exchange. Ordinary query reconnects therefore do not release or silently transfer the claim. Owner EOF releases it; read cuts/candidates/peer clients remain independent. No idle timeout or heartbeat grants or revokes ownership.

The facade's small connection-owning task serializes only claim commands on that channel. While idle it waits for a command or actual stream EOF/protocol failure, without a timer or concurrent readers; caller cancellation does not abandon the task's in-flight checked exchange. The existing bounded operation exchange still has its normal refusal/uncertainty policy. A lost reply retains the old channel until exact recovery or checked close, rather than treating a local future drop as remote release.

### A checked native file lease spans local effect draining across owner restart

Reuse private Directory/lock_file identity checks and explicit OS unlocking for a session-specific lease beneath the canonical project's existing private lock directory. The hashed session identity bounds the filename without changing session identity. The facade acquires it nonblocking before claiming a session and retains its file/directory handles through admitted actor/tool work and shutdown. Its purpose is a local drain barrier across memory-owner generations, not persisted live presence or a second durable catalog. Never delete its file, infer process death from a PID, or reclaim it because time elapsed.

Before a switch, acquire the target lease while retaining the old lease. Refusal drops only the target lease. Keep the old lease until target publication or checked rollback is known. On owner loss, signal the current operation token, stop new dispatch, refuse stale session writes and await existing actor/hook/tool cleanup before releasing native ownership. A same-session competing process cannot acquire the held barrier during that drain even if a successor memory owner has started. Distinct sessions remain independent.

The existing project lock also supplies maintenance exclusion: native drivers retain shared checked ownership while maintenance takes its existing exclusive lock. This does not select a session or project-wide conversation driver; distinct session leases and owner claims do that. It prevents purge/restore from treating a surviving driver's drain barrier as unused merely because its memory owner exited. Retain both native handles through cleanup and never infer their absence by enumerating PIDs or elapsed time.

Using only owner generation checks was rejected: they prevent stale SQL but cannot make a surviving old driver's asynchronous shell cleanup precede a successor claim. Kernel leases are not evidence of remote request completion: already accepted external effects keep their existing journal/uncertainty status, including after process death.

Capture one immutable selected-session lease per admitted invocation, separate from tool identity. Move it into existing retained native owners: Unix shell WorkerRequest beside its root/admission guards, MCP stdio Request/Close command through worker settlement and retained cleanup, and the owning hook/Windows shell cleanup worker. A bounded cleanup refusal or dropped parent future does not release that owner’s hold before actual reap/absence. Idle reusable MCP transports carry no prior-session hold; protocol/HTTP accepted remote effects retain their existing uncertain status rather than a rollback promise. Hook and Windows shell exits that formerly dropped unconfirmed native ownership must retain it on their same owning worker, preserving existing caller budgets and refusal labels. No new cleanup scheduler, platform unsafe API or mutable ToolHost-wide session holder is introduced.

Unix memory-owner election can overlap another session's owned tool launch. Route its existing independent service child creation through the same platform spawn lock. A small typed entrypoint takes the configured Command and optional private stderr file, fixes group0 and null stdin/stdout, and returns the retained std Child. Preserve ServiceProcess observation/reaping and independent owner lifetime; do not turn the service into an OwnedProcessGroup whose parent cleanup kills it. The lock covers only configured stdio and actual spawn, never an exposed callback or child wait. Existing causal WINDOW/BLOCKED test seams must observe an independent second spawn waiting behind owned pipe creation; the affected logged-owner lifecycle fixture must still prove attachment-triggered self-retirement.

Presence EOF with a selected claim but no pending switch is owner loss, not successful selection recovery. Runtime first cancels and awaits owning work while retaining native exclusion, then obtains an explicit checked-existing-instance reopen outcome and a fresh exact catalog/claim in the successor generation. It never replays the completed provider/tool operation. An unresolved old-generation selection remains an explicit uncertain transition; an absent successor claim map does not prove whether it was accepted.

### Exact atomic claim selection and bounded volatile outcome recovery

Use one owner selection operation for initial claim and switch: the request binds authenticated client, expected old claim (or none), target session and full captured target catalog record (or an absent target reservation). Under Shared.write, check the old claim still belongs to this client, check the exact active target/catalog expectation, and check target availability before replacing anything. A typed refusal has no effect and preserves the old claim. Idempotent exact selection uses its existing ServiceRequest UUID as the transition/claim identity rather than issuing another claim or storing a schema receipt.

The dedicated presence attachment retains the latest exact request tuple and selected claim. Reuse ReceiptProgress to distinguish same-generation running/completed/unknown exchanges. A read-only outcome operation verifies the entire client/old/target/catalog tuple and returns the exact selected claim or a proved unchanged old claim only after the handler has completed; unknown, mismatched, disconnected or generation-changed state is not a guessed refusal. The facade retains pending selection before sending and fences driver work/local publication after cancelled or lost replies. No blind resend, release-old-then-claim-new, or durable registration protocol.

If a broken exchange makes the original presence channel unusable, a narrow checked reattachment may move that exact still-owned claim to a fresh dedicated channel only after the original tuple's completed handler and current claim are proved. It changes the connection-ownership identity atomically under the write guard; dropping the old connection removes nothing after that identity changed. This is not new session selection or new authority. A completed no-effect refusal reattaches only the proved old claim; unknown/evicted progress cannot transfer ownership. Do not expose an outcome-shaped mutation as a read-only operation: proof sampling is read-only, exact reattachment is a separately classified volatile resource transition.

If the presence channel is lost, its claim cannot be inferred from a replacement SQL connection. Keep the native lease, cancel/drain the operation, establish exact old-generation handler settlement where possible, and explicitly reacquire the chosen active catalog session on a fresh dedicated connection before later work. Across owner restart, old volatile claims/outcomes are gone: recover existing durable mutation receipts/journals first, then revalidate catalog and claim the selected session under the new generation. Do not label a disappeared transition Committed or NotCommitted from absence alone. Pending local selection stays visible as pending/fenced until authoritative selection or checked restoration resolves it.

Every stale-client recovery/start path must carry the expected existing store instance into the checked startup/lifecycle boundary. The current attach_or_start-then-factory-instance-check sequence is insufficient after narrowing CLI admission: an old client must never initialize a purged store before its later identity refusal. Check existing-instance requirements under the actual election/directory lifecycle ownership, including owner child opens; an unlocked pre-read would be a TOCTOU workaround. Ordinary fresh startup may initialize memory, but recovery of an old driver may only reopen its exact retained instance or refuse.

### Staged runtime selection and real mutation boundaries

Initial resume/continue prepares the existing config/profile/topology and catalog snapshot before final checked claim acceptance, and publishes no provider work or session-only grant context before acceptance. Continue retains its deterministic selected identity; it never skips a busy latest session. A fresh invocation remains a fresh UUID.

For resume/new/fork, retain old selection, permission context and native lease while all target validation runs. A new or forked catalog row may be durably retained if later validation fails; do not silently select it. New session state is prepared separately, written only through the known target driver view with existing exact pending-publication recovery, and published after persistence. An accepted claim followed by a storage failure is a pending/fenced selection, not a usable unchanged old selection. Checked restoration of the old claim precedes reporting an ordinary rollback; retained proof survives cancellation. Permission reset and actor/context publication occur only at the resolved selection boundary.

The existing durable catalog and public chain are authoritative initial state
for a created or forked session. Resume already reconstructs a missing cached
Session record from those checked records. A later new-session cache save uses
the existing pending-publication fence before its write; this split does not
require a new initial-state staging or rollback framework. The target-validation
fixture verifies the retained durable orphan remains unselected on refusal.

Thread current driver proof into session-bound checkpoints, raw actor/relationship appends, summaries/compaction checkpoints and session-record/journal updates. Validate it inside the owning mutation guard before effect or receipt; the proof includes exact owner generation and claim, not merely a client-supplied session string. Shared membership/reports/notes keep their established policy and write behavior, but stale admitted driver work cannot use them to bypass operation cancellation. Catalog rename/fork refuse a different active driver; removal refuses every driven session; generation checks remain. Lifecycle commands consult the owner rather than a UI-only current-session check.

The presence watcher binds the existing shared operation cancellation token before actor/provider/tool dispatch and wakes on actual connection failure or claim/generation rejection. Do not create timer ticks or infer remote rollback. Accepted durable checkpoints still recover from their exact receipts before interruption/notice publication. If local owned cleanup remains unconfirmed, retain its native barrier and report the existing uncertainty; do not admit a replacement by dropping it early.

The recovered dream's Reconciled state is positive exact merge/open-ref proof,
not an already-sent promotion. The explicit inspected-abandonment wrapper must
not call generic reconciliation that would auto-promote it. Unresolved
Reconciling instead returns the existing typed resolution-required refusal.
For Reconciled, retain a checked current-session invocation hold and the existing
dream lease, settle independent memory effects, then use existing exact
branch/base/head abandonment. An already-sent abandonment remains fenced until
its typed request resolves; Pending/Confirmed promotion recovery is unchanged.
No generic reconciliation-intent abstraction, protocol or schema is added.

### Bounded presence projection and compatible delivery

LiveSessions returns bounded claim metadata only. One claim per dedicated attachment bounds the full live inventory by the owner's existing attachment ceiling; it introduces no new session or product graph cap. No private history, provider data, PID, client credential or prompt is exposed. Runtime joins presence once with the existing catalog listing. A cold standalone inspector reports unknown presence; an attached compatible owner reports known current metadata. Keep CLI's existing array shape and picker behavior, with actionable already-driven/changed/pending diagnostics and no provider call on refusal.

Advance one final coordinated protocol minor and wire golden from N3 1.11, including the handshake compatibility floor needed to stop older writable callers bypassing claim checks. Maintenance still uses the existing project lock and memory-owner exclusion; conversation/run/serve/dream use session admission, while free undo uses the existing dream lease. Immediate last-attachment retirement is unchanged.

## Operational surface

This runs in the existing native local application and per-project memory owner on supported macOS, Linux and Windows targets, not a container/provider service. Keep the existing private Unix socket or Windows named-pipe endpoint; no TCP bind address, public ingress, new environment secret or provider credential route is introduced. The endpoint's existing private authority authenticates connections; client UUID is ownership metadata and is omitted from user-facing listings. Native checked lock files stay under the canonical project's private data/locks directory, outside tool roots.

Keep the owner's existing attachment ceiling and client's existing ordinary connection bounds; no increase is authorized here. A driver adds one dedicated presence connection; live inventory is bounded by that actual resource ownership. Idle presence waits for connection EOF, not a new timeout; existing bounded handshake/frame/query policies remain. Supported binary architectures, pinned Rust/mise/Cargo dependencies and bundled full-Dolt version/inventories remain unchanged. One coordinated private protocol minor and writable compatibility floor change together with the wire golden and actionable older-owner/client refusal.

User flow is ordinary launch or explicit resume/continue → prepare catalog/topology and native exclusion → owner-checked claim → publish selection/grant boundary → admitted turn. Busy/changed selection returns the exact existing choice and actionable refusal without provider activity. Lost reply shows pending ownership and fences work until recovery. Listings/picker project safe known/unknown presence; process shutdown drains locally owned work, releases claim/attachments and triggers immediate final-client retirement.

## Risks / Trade-offs

- [Loss arrives during accepted work] → cancel future dispatch, preserve exact receipt/journal recovery, await local cleanup while retaining the native barrier; report accepted external effects honestly.
- [Lost switch reply] → retain the exact tuple and both relevant leases; fence selection and query existing owner progress instead of treating EOF as handler completion.
- [Owner restart] → old claim is invalid, surviving driver retains its drain barrier, and new claim requires fresh catalog validation after existing durable recovery.
- [Selection and permission publication diverge] → stage all fallible preparation; no self.session assignment before persistence, and unresolved acceptance remains explicit rather than pretending old selection is usable.
- [Parallel U2 edits] → separate worktrees and normal stable-commit integration, preserving its tool loop cancellation and pure pinned-view metadata seam before final checks.
- [Lease/drop cleanup fails] → retain checked handles/unconfirmed ownership and report failure; no stale-PID kill, lockfile deletion or timeout-based release.
