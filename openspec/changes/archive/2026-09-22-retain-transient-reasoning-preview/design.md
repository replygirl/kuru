## Context

`ProgressObserver` already maintained a bounded `summary_tail` from streaming
`ReasoningSummaryDelta` text and the TUI rendered it only while the selected
speaker's turn was active. P08 removed that path while adding durable settled
summary records, conflating an ephemeral text-only preview with the records'
private payload and coordinates.

## Goals / Non-Goals

**Goals:**

- Restore the existing bounded selected-speaker preview.
- Keep settled records, provider coordinates, transcript, peer context, and
  durable event detail separate from that preview.

**Non-Goals:**

- Add a new display surface, API, persistence field, or replay policy.
- Expose settled summary records through public event streams.

## Decisions

- Reuse `ProgressObserver::summary` and its existing 2 KiB tail bound only for
  streaming `ReasoningSummaryDelta` text.
- Continue ignoring `SettledReasoningSummaries` in `ProgressObserver`; actor
  persistence remains its separate post-settlement path.
- Keep the preview's text and truncation state in the active turn's
  `FacingProgress`, which is cleared when that turn completes.

## Risks / Trade-offs

- [Risk] A later consumer mistakes preview text for durable private state.
  → The settled event remains ignored by progress, no coordinates are copied,
  and focused tests distinguish the active preview from private persistence.

## Operational surface

The change has no listener, secret, binary-version, or connection-limit change.
It only restores the existing selected-speaker preview inside the already-active
runtime/TUI process.
