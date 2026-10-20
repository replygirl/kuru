# Design

## Context

Live replies are labeled in `View::complete_turn` through `View::actor_name`, which reads the view's `parts` roster and `relationships`. After each persisted turn, on session restore, and when pages load, `apply_public_read` installs a page built by `Transcript::from_page`, whose `append_turn` uses `record.speaker_id` as the label. That static projection cannot see the roster, so the persisted page replaces the correctly labeled live items with UUID-labeled ones. The newest reply looks right only until the next page install.

## Goals / Non-Goals

**Goals:** persisted and live entries get one label; roster changes relabel earlier entries; one fallback for unknown ids.

**Non-Goals:** changing `PublicTurnRecord` or migrations (a stored display-name snapshot is a follow-on), notice/local-item placement, and the record-level outputs that still print the stored speaker id: `kuru sessions`, `/export`, `kuru sessions export`, `kuru run --json`, `kuru memory export` and the fork-boundary picker.

## Decisions

- Extract the roster lookup into a free function `speaker_label(parts, relationships, id)`; `View::actor_name` delegates to it. Rejected: a second lookup in the transcript module, which would drift.
- Page projection takes `&dyn Fn(&str) -> String` (`from_page`, `install_newest`, `install_older`, `install_newer`). Call sites in `apply_public_read` pass a closure over `view.parts` and `view.relationships`, which borrows disjointly from `view.transcript`. The initial view projects the runtime snapshot first and labels with its roster. Rejected: storing a roster inside `Transcript`, which would duplicate view state.
- `Item` gains `speaker: Option<String>` with the raw id; `(label, text)` tuples stay the rendering projection. `Transcript::relabel` re-derives the label of every item with a speaker, giving a changed item a new `projection` id so cached layouts rebuild. `apply_runtime` and `Event::Relationship` call it. A relationship reply also keeps its speaker id (the relationship id). `push_speaker` takes the id with the reply, so no later call has to tag the newest item.
- `transcript_label` wraps `speaker_label` for transcript replies: a relationship known to the view reads `kind · members` (members named by the roster), exactly as `complete_turn` builds it through the shared `relationship_label`. `actor_name`, used by activity and routes, keeps the kind-only fallback. Rejected: changing `speaker_label` itself, which would alter activity text.
- `apply_runtime` relabels only when parts or relationships actually changed, so no-op snapshots do no work.
- Unknown ids use the existing 16-character prefix on both paths. A record without a speaker keeps `unknown`.

## Operational surface

Interactive presentation only: no bind address, container, secret, connection limit or binary/architecture change. The label function reads only the view's already-public roster and relationship names and the public record's speaker id; no private history or provider data is involved.

## Risks / Trade-offs

- Exception: a relationship that is no longer in the view's relationships (dissolved, or a member left the roster) cannot be rebuilt, so its replies, live or reloaded, show the first 16 characters of the relationship id. The composite is derived from current membership, not the membership at the time of the turn.
- Relabel runs over at most two retained pages plus bounded locals on each runtime snapshot, which is cheap.
- Names reflect the current roster, not the roster at the time of the turn. A retired part's earlier replies fall back to the id prefix until memory stores a display name (follow-on).
