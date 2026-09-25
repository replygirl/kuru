# actor-context-compaction Specification

## Purpose
Define how Kuru compacts each actor's admitted private context into durable, bounded summaries while preserving raw history, policy boundaries, exact accounting and session identity. It also specifies automatic threshold behavior and the working manual `/compact` control.

## Requirements

### Requirement: Actor compaction preserves exact private source boundaries

Kuru SHALL compact each part or relationship independently from the raw-history source admitted by its effective visibility and memory policies. The source MUST bind the actor namespace, session identity, pinned live or candidate view, captured revision, exclusive prior cursor and inclusive last sequence. It MUST exclude another actor's raw history, another session's raw history, unattributed legacy rows, public transcript material the policy omitted, and provider-supplied P08 reasoning summaries. Original source rows SHALL remain stored without expiry or rewriting.

#### Scenario: Simultaneous sessions compact one actor
- **WHEN** two sessions compact the same actor while each contains a distinct sentinel and one unattributed legacy row exists
- **THEN** each summary and cursor covers only that session's admitted sequenced rows, neither summary contains the other sentinel or legacy row, and every raw row remains inspectable.

#### Scenario: Relationship and part stay independent
- **WHEN** a relationship and one of its member parts both need compaction
- **THEN** each uses its own admitted private source and summary namespace, with no member history copied into the relationship except an explicitly supplied contribution already allowed by policy.

### Requirement: Compaction publishes one rolling summary and cursor atomically

For one actor/session/source, Kuru SHALL combine the prior cursor-selected compaction summary, when present, with the next bounded sequenced raw-source prefix and request one bounded replacement summary. It MUST publish the strict operation-attributed context record and advance the source cursor through the P29 conditional checkpoint only after the provider invocation has settled successfully. The record MUST retain actor, session, source and summary namespaces, source view and historically verifiable revision, exclusive and inclusive sequence bounds, and real producer-actor, operation and invocation identities without fabricating a conversation turn. Publication MUST atomically verify that the exact consumed row contents/order/count and selected prior cursor/summary still match the recorded revision on the pinned view; an unrelated peer's project revision advance MUST NOT make an unchanged source stale. Any settled producer-private reasoning returned by that accepted Compact invocation MUST commit in the same checkpoint and MUST NOT enter later compaction or replay context. A failed, cancelled, invalid, stale, conflicting or uncertain attempt MUST NOT make a new summary, cursor or sidecar usable; reconciliation of an accepted lost reply MUST settle the same atomic mutation without a second provider invocation. A provider invocation that settled before a stale publication result MUST remain charged once without provider replay.

#### Scenario: Relevant source changes during summarization
- **WHEN** a sibling changes a consumed row or selected prior cursor/summary before the summary checkpoint
- **THEN** the typed stale result leaves the previous summary and cursor usable, publishes no partial summary, retains the settled provider usage once, and does not fence unrelated later work as an uncertain mutation.

#### Scenario: Unrelated peer advances the project revision
- **WHEN** another actor writes its own private history while this actor's captured source and prior summary remain unchanged
- **THEN** the exact historical source proof permits this actor's summary and cursor to publish without mixing either actor's private rows.

#### Scenario: Accepted checkpoint reply is lost
- **WHEN** the exact summary checkpoint commits but its reply is lost
- **THEN** managed receipt reconciliation proves the one committed summary and cursor, and exact retry makes no second summarization request or duplicate row.

### Requirement: Automatic compaction is bounded and non-recursive

Kuru SHALL expose a documented bounded `context_compaction_threshold_percent` and `context_compaction_output_reserve_tokens`. Before P7 omits optional history for an ordinary actor request, it SHALL compare the exact full policy-admitted candidate request estimate to the selected model window and automatically compact only that actor when the configured fraction is reached. Ordinary request omission MUST NOT hide a reached threshold. The compaction request MUST retain its instruction, prior usable summary, selected whole source rows, output reserve and accounting envelope, and MUST choose the largest oldest source prefix that fits without splitting a row. A compaction invocation MUST NOT recursively trigger compaction. At most one automatic compaction attempt per actor request is allowed; afterward Kuru SHALL rebuild the ordinary request, and if its mandatory input, active tool chain, native continuation, retained summary and reserve still exceed the hard window it MUST refuse before dispatch rather than drop mandatory material. P7 MAY still omit remaining optional whole rows from the rebuilt ordinary request under its existing contract.

#### Scenario: Threshold compaction makes room
- **WHEN** optional raw history brings one actor's effective request to the configured fraction and a bounded source prefix fits the compaction request
- **THEN** one accounted summary request settles, the ordinary request is rebuilt from the new summary plus remaining admitted context, and the original rows remain stored.

#### Scenario: Mandatory material still overflows
- **WHEN** current explicit input, active receipts or continuation, the retained summary and output reserve still exceed the hard window after the one allowed compaction attempt
- **THEN** Kuru refuses before the ordinary provider dispatch and does not recurse, delete a receipt, truncate mandatory material or advance another cursor.

### Requirement: Compaction continuity is policy-selected and current

Later actor requests SHALL use current-session raw rows after the durable cursor plus the latest cursor-selected summary for that actor/session/source. Cross-session compaction summaries MAY enter context only through the active mode's explicit visibility and memory-policy selection, using the bounded P29 cursor-selected summary projection; superseded historical summaries MUST NOT enter ordinary context. Candidate work SHALL read and publish only on its pinned candidate view. A summary is context maintenance and MUST NOT be represented as a belief, note, public transcript message or provider reasoning block.

#### Scenario: Policy admits one cross-session summary
- **WHEN** the effective mode admits settled summary continuity for an actor but does not admit its other session's raw history
- **THEN** the latest admitted summary may appear with current-session rows, while the other session's raw rows, superseded summaries and P08 reasoning records remain absent.

#### Scenario: Candidate compaction conflicts
- **WHEN** a dream candidate compacts its actor source while live main changes
- **THEN** the summary remains on the candidate, promotion follows the existing exact-base conflict rules, and neither live context nor another candidate observes the unpromoted summary.

### Requirement: Manual compaction reports retained-source truth

The working `/compact [ID]` command SHALL run through the shared command registry without becoming an ordinary provider turn. With an identity it SHALL compact only that live part or relationship; without one it SHALL visit the stable ordered active identities independently. It SHALL report, per attempted identity, the exact summarized source range or an actionable no-op/refusal, the resulting summary identity when settled, and that original records remain stored. Cancellation MUST drain the current owned request and leave later identities unstarted.

#### Scenario: Manual compaction completes
- **WHEN** a user runs `/compact` with eligible histories for two active identities
- **THEN** the command emits settled notices in stable identity order, each notice names its own covered range and retained originals, and no public transcript or note is rewritten.

#### Scenario: Manual compaction is cancelled
- **WHEN** the user cancels while one manual summary request is active
- **THEN** that invocation settles as cancelled or reconciles its exact accepted checkpoint, no later identity starts, and every previous usable summary remains valid.
