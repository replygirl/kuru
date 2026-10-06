# Design

## Context

`Event::ToolStarted` and `ToolSettled` already represent runtime admission/settlement. The public wire intentionally contains no result body, and its historical start record lacks a call ID. `ToolObservation` has projected arguments/outcome/bytes/digest but cannot provide external output or the connector's invocation-bound file receipt identity. `CheckpointLease::snapshots` already validates private retained receipt bytes. `ShellCapture` finalizes its result and diagnostic only after EOF; the scanner can release safe bytes earlier.

## Goals / Non-Goals

Implement the proposal's current-session cards with a narrow tool-specific private presentation seam. Selectively reuse the old P15 diff/card/preview algorithms while retaining present composer and notice ownership.

No memory table/migration, persisted card bodies, durable event/outbox framework, journal/TurnOutput extension, public raw result, provider route, new tool effect/budget, or N3 dream policy. Original checkpoint/history data stays durable and untouched; transient presentation availability is explicit.

## Decisions

### Admission context remains runtime-owned

Capture the final rewritten call context where runtime admits it, including session/turn/actor/invocation/call and a monotonically assigned admission ordinal. Obtain file receipt identity from the connector's existing checked identity function rather than replicating its hash. Emit a private typed start/settlement projection alongside existing metadata events. Reject deriving cards from streaming progress or matching only tool name, which misattributes same-name calls and provisional proposals.

### Retain a bounded transient detail window

Retain at most 64 detailed cards and at most 1 MiB serialized presentation data; bound each result at the existing 8 KiB runtime projection and partial stdout/stderr previews at 2 KiB each. Eviction/coalescing is visible as unavailable/omitted detail and does not alter dispatch or durable history. Exact identity and settled-state precedence prevent late previews from reviving a completed call. Session/view changes clear transient ownership. Replay uses existing known metadata only and explicitly withholds missing bodies/order/receipt identity. Reject the old dedicated SQL card/history/export bundle: it exceeds the assigned product scope and would introduce new storage recovery contracts.

Charge preview handles, overlapping terminal/UI stream snapshots and expanded diff reserves against that window. Feed filtering compares the exact retained session owner, separately from redacted display identifiers. Operation return or actual aborted-turn recovery marks any missing transient settlement as interrupted without inferring a tool outcome. Capture the selected view using its already-pinned pure metadata getter after the completed-turn retry short-circuit; presentation must not add a failure or SQL/RPC read between durable admission and finalization.

### Content exposure is field-aware

Internal cognitive/peer tool arguments and results are metadata-only. External results use the existing connector/runtime projected receipt with no new raw result path; if arguments are shown, only bounded redacted external arguments cross the private projection. Fixed identifiers and statuses provide useful cards when content is withheld. Redaction alone does not authorize private actor-context disclosure.

### Diff is a connector projection of recorded snapshots

Add a read-only `CheckpointStore::diff` and ToolHost/runtime forwarding over `lease().snapshots(id)`. Reuse the old linear prefix/suffix whole-line changed-region projector: at most 2 MiB per side, 32,768 lines, 8 KiB output, explicit binary/oversize/uncertain/missing/pruned states. Redact whole checked text before selecting a hunk. Never read a later workspace file, change the private receipt or copy full snapshots into card storage.

The TUI retains a read-only checkpoint reader and runs at most one owned blocking read outside the interactive loop and Harness lock. Tag the result with exact session, card and receipt, ignore stale results after clear/session changes, use a fixed unavailable label on errors, and await the owned read before releasing private roots. Completion schedules the next eligible expanded receipt; receiptless shell/cognitive cards cannot block file diffs. Distinguish each View (including clones) in the existing transcript cache without redesigning layout.

### Safe previews stay separate from EOF results

Expose scanner-released safe head/tail through a concrete ShellProgress handle with independent bounded stdout/stderr slots. Producer publication uses try_lock with visible omitted-update counts; one operation consumer receives owned snapshots and coalesced Notify wakeups, never a guard to hold through drawing or await. Preserve scanner state and coalesce latest updates without pipe backpressure. The regular capture still finishes only on EOF, and incomplete operational stderr remains pending EOF. Cancellation unregisters preview ownership after existing controlled cleanup; late updates cannot settle a card. Reject raw-chunk callbacks or premature `finish`, which leak cross-read secrets and weaken existing diagnostics.

### Preserve UI and integration ownership

Adapt old card rendering to current cached transcript lines, stored scroll clamp and U3 modal/grapheme controls. Use existing completed-frame test synchronization at 80/120. Preserve C1 confirmed notices and event settlement drains. N3 independently owns dream/reconciliation engine sections; integrate its stable changes normally before delivery, without overlapping writes or a second protocol change.

## Risks / Trade-offs

- Transient output disappears on reopen → show explicit unavailable detail and reuse only known recorded metadata without guessed replay or effects.
- Noisy or concurrent calls overwhelm the presentation consumer → count/byte bounds, coalescing and visible omission; execution/cleanup never wait for drawing.
- Cognitive content resembles ordinary tool output → classify at runtime before creating any content-bearing projection and verify fake private sentinels absent from whole frames/output.
- A small late edit follows a secret opener → full-input redaction precedes bounded changed-line extraction.
- New preview hooks accidentally change stderr diagnostics → retain EOF-only capture and run existing timeout/cleanup regression cases.

## Operational surface

The existing local Kuru TUI, connector ToolHost and package-owned native fixtures are the execution surface; no listener, container, runner topology, required credential, runtime engine version or supported architecture changes. HTTP fixture providers bind loopback ephemeral ports with fake secrets and demonstrate local behavior only. Existing tool permissions, trust and checked workspace root remain prerequisites. Presentation retains the fixed count/byte limits above, and shell preview consumers cannot affect process/pipe cleanup. Native Windows, coverage and paid live-model outcomes are separate acceptance evidence, never inferred from target compilation or fake HTTP providers.
