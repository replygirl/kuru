# Design

## Context

Live replies are labeled in `View::complete_turn` through `View::actor_name`, which reads the view's `parts` roster and `relationships`. After each persisted turn, on session restore, and when pages load, `apply_public_read` installs a page built by `Transcript::from_page`, whose `append_turn` uses `record.speaker_id` as the label. That static projection cannot see the roster, so the persisted page replaces the correctly labeled live items with UUID-labeled ones. The newest reply looks right only until the next page install.

## Goals / Non-Goals

**Goals:** persisted and live entries get one label; roster changes relabel earlier entries; one fallback for unknown ids.

**Non-Goals:** changing `PublicTurnRecord` or migrations (a stored display-name snapshot is a follow-on), notice/local-item placement, and the record-level CLI outputs.

## Decisions

- Extract the roster lookup into a free function `speaker_label(parts, relationships, id)`; `View::actor_name` delegates to it. Rejected: a second lookup in the transcript module, which would drift.
- Page projection takes `&dyn Fn(&str) -> String` (`from_page`, `install_newest`, `install_older`, `install_newer`). Call sites in `apply_public_read` pass a closure over `view.parts` and `view.relationships`, which borrows disjointly from `view.transcript`. The initial view projects the runtime snapshot first and labels with its roster. Rejected: storing a roster inside `Transcript`, which would duplicate view state.
- `Item` gains `speaker: Option<String>` with the raw id; `(label, text)` tuples stay the rendering projection. `Transcript::relabel` re-derives the label of every item with a speaker, giving a changed item a new `projection` id so cached layouts rebuild. `apply_runtime` and `Event::Relationship` call it. A live reply with a relationship label carries no speaker id, so it keeps its composite label until the page reinstalls it.
- Unknown ids use the existing 16-character prefix on both paths. A record without a speaker keeps `unknown`.

## Operational surface

Interactive presentation only: no bind address, container, secret, connection limit or binary/architecture change. The label function reads only the view's already-public roster and relationship names and the public record's speaker id; no private history or provider data is involved.

## Risks / Trade-offs

- Persisted entries of a relationship turn are labeled with the stored speaker (the relationship id), not the live composite `kind · members`. This matches the previous persisted behavior in kind; only the id becomes a name.
- Relabel runs over at most two retained pages plus bounded locals on each runtime snapshot, which is cheap.
- Names reflect the current roster, not the roster at the time of the turn. A retired part's earlier replies fall back to the id prefix until memory stores a display name (follow-on).
