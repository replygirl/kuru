# Design

## Context

The current initial projection requests 500 public records, discards page cursors and identities into speaker/text tuples, and scrolls a full wrapped-line cache by u16 distance from the tail. U2 cards anchor to vector indices. The existing storage API already supplies exact selected-session public turn/legacy identities, head/revision provenance, response bounds and fork-prefix validation. However, `store.rs::public_transcript_page` holds `Shared.write` while loading every complete settled turn on every page; a 1,025-turn session at 32 records per page requires 33 full chain walks. Response size is not a bound on this storage work.

## Goals / Non-Goals

**Goals:** Meet the new capability through concrete TUI projection, navigation and terminal ownership seams; prove real long-session responsiveness and source isolation before claiming bounded paging solves performance.

**Non-Goals:** Retain the exclusions in proposal.md. In particular, no durable SQL cards, private history search, generic data/input framework, new dependencies, U4 themes, changed actor behavior or guessed title restoration. A memory optimization is evidence-driven and coordinated, not implicit permission to redesign storage.

## Decisions

### Immutable read pool and metadata-only chain proof

The existing 1,025-turn fixture passed at 32 records per page with exact coverage and rejection checks. Source inspection establishes the repeated full-body walk and shared writer-lock cost; it is not a benchmark or an additional gate before app implementation.

Keep `public_transcript_page` and the current exact HEAD revision invalidation. Capture the selected attachment's actual HEAD under a short shared writer guard and validate any cursor against that captured revision. Never select a pool from a cursor hash. Release the guard before opening the existing commit-qualified pool, prove its exact revision and validate its historical schema as the existing immutable read-cut implementation does. Read catalog, pending state, chain and legacy prefix in a transaction on that immutable pool.

Traverse structural public-turn metadata to prove node identities, origin/turn/kind coordinates, settled predecessors, reachability and exact counts. Only requested page bodies (and pending projection) are decoded through the existing typed public-message/private-reasoning validation and byte budgets. Unreturned bodies are not fetched or decoded. This removes repeated body work and keeps traversal outside the mutable writer mutex; metadata traversal still repeats per page and a whole-history scan remains quadratic in metadata query work. Do not claim this is a linear search implementation. Actual reader/other-session writer acceptance must establish the ownership boundary; native query batching may refine evidenced usability later without new schema/index/protocol authority.

Rejected: trusting cursor revision as pool authority, retaining the writer lock through a cold historical pool open, accepting unreachable nodes, removing returned-body validation, changing cursor semantics, or treating response bounds as a storage-work proof.

### Concrete public projection adapter and bounded view window

The existing app adapter captures the canonical selected session and pure selected-view name from Harness, then performs public-page reads outside its mutex. Keep MemoryStore out of UI/render models. Retain at most two adjacent 32-record response pages, each under the existing source byte envelope, and one owned in-flight response/read. Page pending state belongs only to the newest projection. Separately bound current-operation/local display entries and expose their visible omission accounting without deleting stored history. Do not introduce a product cap on legal session size or legal stored messages.

Use actual turn node plus projected message slot, or actual legacy sequence, as durable item identity; use local ephemeral identities for notices. Retain page entry cursors and exact captured provenance. Walking back from the head to recover an evicted newer location is bounded page-at-a-time work, not an unbounded retained cursor history. `/clear` advances the view epoch so old results cannot silently repopulate its cleared surface.

Rejected: arbitrary tuples as identity, full-session loading, forged cursors, unbounded page/cache retention or reading private namespaces to rebuild public content.

### Item anchors, viewport projection and precise transient card binding

Represent navigation as FollowTail or Reading(item identity, wrapped row). Cache layout only for bounded retained items and invalidate changed item/card layouts or width changes. Produce styled viewport rows rather than retaining every wrapped line of all source bodies; long individual items remain navigable through bounded layout checkpoints/row projection. Clamp a shrinking item locally. U2 cache identity remains fresh per View, and exact session/turn/view binding determines a card's associated user item; redacted display IDs never authorize attachment.

Rejected: history-wide u16 offsets, repeated whole-session tuple comparisons on each frame, stable indices across page eviction, or truncating legal stored text solely to make a renderer cache fit.

### One owned incremental search/read job

Use one concrete owned page/search job and one coalesced pending request, tagged with session/view/query epoch. Search walks selected public messages case-sensitively, retaining bounded scan progress, one hit/continuation and the original draft/anchor. Each page result passes epoch/provenance checks before publication. Cancellation is an honest partial state; a cursor revision error is stale/unavailable, never completed no-match. Historical cards with unavailable transient bodies are excluded. Shutdown awaits the owned read cleanup; no Harness mutex spans an entire scan.

Rejected: spawning unbounded searches per keystroke, FTS/private memory search, relabeling old cursor revisions, or claiming an immutable scan across source writes without a storage proof.

### Modal priority and paired terminal state

Ctrl-F opens transcript search only after existing approval/picker/recall priority. Search edits its own literal query and Escape restores the saved composer draft/anchor. Existing composer Home/End and card keys retain their semantics. Mouse wheel navigates the appropriate visible surface under the same priority and capture is on by default.

The terminal guard pairs mouse with existing focus/paste/raw/alternate-screen state. Advertised xterm title stack support permits saving the actual window title with CSI 22;2 t and restoring with CSI 23;2 t; skip unsupported mechanisms. Emit only bounded sanitized session title content. See the official [xterm control sequences](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs-contents.html). Fixed BEL is a content-free capability-dependent signal, not a promise of desktop notification. Emit it only after the current successful nonreused turn settles while unfocused, never from arbitrary activity events.

Rejected: setting a guessed default title, OSC content from prompts/answers, notification on replay/error/cancellation, or a new terminal capability framework.

## Integration Contract

Inputs are the existing selected-session public page/cursor and exact admitted tool binding; outputs are public projected items, bounded navigation/search state and terminal writes. Reads never acquire mutable candidate authority, change a session, replay inference or alter journal/Event/TurnOutput schemas. Cursor, job and view epochs are checked before applying results. Session switch/clear invalidates old work; final loop cleanup retains and awaits any owned read before terminal restoration. Existing C1 activity/notice settlement ordering and U6 headless signal handling remain unchanged. N4 picker/admission integration occurs through stable commits and exact shared-function coordination.

## Operational surface

The existing native `kuru` TUI runs in its caller's real terminal on the supported target architectures. It uses the existing checked private project memory attachment and shipped Dolt engine through package-owned preparation; there is no new listener/bind address, container, secret, dependency or binary version requirement. One owned public read/search job and one coalesced pending request bound interactive connection work. Existing configured providers and tool authorities activate through their existing trust/lifecycle paths, independently of read-only public navigation. Terminal mouse/focus/paste acquisition is paired by the guard; title support is restricted to a restorable advertised mechanism. Completed-frame 80/120-column PTYs provide local terminal evidence; supported-target native and coverage acceptance remains part of normal later CI.

## Risks / Trade-offs

- [Repeated metadata traversal remains quadratic across pages] → Keep body decoding page-bounded and reads on a proven immutable pool outside the writer mutex; state the remaining cost plainly and refine native queries only when actual navigation requires it.
- [Live revisions invalidate a search continuation] → Show stale partial progress and permit an explicit fresh scan; never claim complete no-match or fabricate cursor provenance.
- [A legal individual public item can be large] → Bound retained pages and viewport/layout state separately; test long items rather than dropping their text or building a full wrapped-line vector.
- [Evicted newer pages need additional reads] → Recover locations incrementally with bounded state and visible progress instead of retaining every page/cursor.
- [Terminal capabilities vary] → Default mouse uses the existing guard; title changes require a supported restoration mechanism and completion signaling remains explicitly capability-dependent.
