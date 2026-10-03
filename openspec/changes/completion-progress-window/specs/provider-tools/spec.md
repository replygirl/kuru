# Spec Delta

## ADDED Requirements

### Requirement: Completion progress and silence bounds

A model completion SHALL remain bounded by the 600-second completion deadline
and SHALL otherwise fail only when its provider has made no progress for the
300-second stream silence budget. The Responses stream MUST fail when no wire
chunk arrives within that budget. The runtime's turn and context compaction
model calls MUST each use the same silence budget, restarted by every provider
event the call observes, and MUST enforce the 600-second completion deadline
themselves, so a provider outside the connector's HTTP client is bounded too.
The runtime MUST NOT impose a shorter flat bound on a completion that keeps
making progress. Forwarding provider events to the runtime's observers for this
purpose MUST NOT alter, drop or reorder them. A silence failure MUST name the
model call and the silence; a deadline failure MUST name the model call and the
completion budget; a provider failure MUST be returned unchanged. Cancellation
MUST still end the call before either bound.

#### Scenario: Completion streaming past three minutes
- **WHEN** a provider delivers events at intervals shorter than the silence budget for longer than 180 seconds and then completes within the completion deadline
- **THEN** the turn or compaction model call succeeds and its observers receive every event unchanged

#### Scenario: Provider falls silent
- **WHEN** a provider delivers an event and then none for the silence budget
- **THEN** the model call fails one silence budget after that event with a message naming the call and the absence of provider progress

#### Scenario: Stream that never settles
- **WHEN** a provider keeps delivering events within the silence budget but never completes
- **THEN** the model call fails at the 600-second completion deadline with a message naming the call and the completion budget
