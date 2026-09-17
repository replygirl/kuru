## ADDED Requirements

### Requirement: Durable per-invocation accounting

Kuru MUST durably admit one bounded, stable-identity record before each provider dispatch, including deliberation, speaking, consultation, continuation and dream calls. Ordered usage observations and the terminal report MUST update that same record, and settlement MUST record success, failure or cancellation without manufacturing missing usage. Duplicate delivery of identical records or observation sequences SHALL be idempotent; a conflicting duplicate MUST fail. Completed exact retry MUST add no invocation. An admitted invocation with no final report SHALL remain visibly incomplete after restart.

#### Scenario: Failed request reports partial usage
- **WHEN** a provider reports usage and later fails or is cancelled
- **THEN** the observed components remain durable, missing components stay unknown, and the invocation is incomplete rather than counted as a successful turn.

#### Scenario: Reconciled duplicate
- **WHEN** a durable-write reply is lost and the same invocation identity and observation are retried
- **THEN** one record remains with one copy of each observed component; a changed payload under the same identity is rejected.

### Requirement: Candidate-independent operational ledger

Usage SHALL live on a per-project permanent operational Dolt branch outside live `main` and dream candidates. A usage commit MUST NOT advance the live revision, enter candidate content, alter promotion eligibility, or be lost when a candidate is abandoned or rejected. The branch SHALL be opened and validated before provider dispatch and follow the owned memory lifecycle; no general writable branch handle SHALL escape `kuru-memory`.

#### Scenario: Open candidate and usage
- **WHEN** a dream candidate remains open while its provider usage is durably observed
- **THEN** the live revision is unchanged, the candidate can still promote when otherwise valid, and usage appears once whether that candidate promotes or is abandoned.

#### Scenario: Invalid operational branch
- **WHEN** the ledger's owned schema or receipt contract cannot be validated or migrated at writable open
- **THEN** no unaccounted new provider work begins, while historical inspection reports its uncertainty rather than substituting live-branch totals.

### Requirement: Honest session totals and frozen estimates

Session totals SHALL fold per-invocation records, summing reported input and output tokens only; cached input and reasoning output are subsets and MUST NOT be added again. Each component SHALL distinguish missing from explicit zero, and a terminal component SHALL override earlier progress while retaining earlier evidence for terminal-missing components with an incomplete label. Price schedule, basis and source SHALL be frozen at invocation time; later catalog updates MUST NOT reprice history. Subscription pricing SHALL be labelled API-equivalent estimate, never an invoice, quota or known zero. A resumed session lacking its new-session ledger marker SHALL permanently disclose incomplete pre-ledger history.

#### Scenario: Incomplete resumed session
- **WHEN** `/cost` inspects a session created before ledger markers existed
- **THEN** known current records are shown, but historical totals remain labelled incomplete even if the earlier turn count was zero.

#### Scenario: Price and usage gaps
- **WHEN** a report lacks cached tokens, terminal usage or a verified price component
- **THEN** the corresponding component or estimate is unknown/incomplete rather than zero; a later catalog price change does not revise the stored estimate basis.

### Requirement: Effective-request context fit

Before each inference dispatch, Kuru SHALL estimate the selected effective connector request after native pending continuation and current receipts are assembled. Instructions, tool schemas, current user input, required receipt/call chain and native pending bytes SHALL be mandatory. It MAY omit only older optional history rows as whole units, without mutating durable history, and SHALL report actual omitted row counts and source labels. If mandatory material plus output reserve exceeds the selected model window, it MUST refuse before network dispatch. Unknown model windows SHALL use a documented, visibly labelled conservative assumption rather than block inference solely for missing metadata. Byte transport limits remain independently enforced.

#### Scenario: Oversized native continuation
- **WHEN** encrypted pending continuation and matching tool results exceed a small configured window even though visible messages alone appear to fit
- **THEN** the connector refuses before HTTP, preserves the actor-private continuation, and neither tool results nor opaque bytes are silently dropped.

#### Scenario: Optional history is omitted
- **WHEN** old visible history must be reduced to fit a request
- **THEN** complete older rows are omitted only from that request, current receipts remain intact, persisted rows remain unchanged, and the reported omission count equals the rows actually omitted.
