# Design

## Context

The native Windows failure occurred during isolated fixture deletion. The generic
stale error does not identify whether the manifest transition, chunk deletion or
manifest deletion failed. The fixture also drops its read lease before deletion
and reports cleanup before its functional result.

## Decisions

Keep the same acquired alias lease across `read_locked` and exact-generation
deletion. Collect cleanup errors while cleaning both aliases and shutting down
the server, then retain the preceding functional error as the primary cause.
Add only fixed operation labels to native vault errors; preserve their sources.

## Integration contract

The real native store contains only this fixture's synthetic records bound to its
isolated project. Its generation checks, manifest recovery and native route stay
unchanged. No user credential contents, accounts or resource URLs enter labels.

## Risks / Trade-offs

The observed Windows backend cause remains unproved. A host pass proves local
behavior only; fresh Windows CI is required to observe a recurrence or corrected
native execution. Do not replace that requirement with a retry or weakened guard.
