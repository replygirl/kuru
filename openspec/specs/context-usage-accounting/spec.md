# context-usage-accounting Specification

## Purpose
Account for every provider invocation durably and check the complete serialized request against its context budget before dispatch. Preserve reported usage across failures and dream candidates, disclose missing usage and estimated prices honestly, and explain context omissions without deleting stored conversations or memory.

## Requirements

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

Session totals SHALL fold per-invocation records, summing reported input and output tokens only; cached input and reasoning output are subsets and MUST NOT be added again. Each component SHALL distinguish missing from explicit zero, and a terminal component SHALL override earlier progress while retaining earlier evidence for terminal-missing components with an incomplete label. Price schedule, basis and source SHALL be frozen at invocation time; later catalog updates MUST NOT reprice history. Subscription pricing SHALL be labelled API-equivalent estimate, never an invoice, quota or known zero. A resumed session lacking its new-session ledger marker SHALL permanently disclose incomplete pre-ledger history. An incomplete money estimate SHALL name each priced term it did not apply, including a long-context tier and a cache-write rate, and a term the fold cannot decide SHALL stay unapplied rather than be applied in part. Each record SHALL retain its raw per-kind token components and its frozen price basis so a later reading can apply a term this version does not price. A reported history or evidence gap SHALL remain its own disclosure and MUST NOT be reported as an unapplied priced term.

#### Scenario: Incomplete resumed session
- **WHEN** `/cost` inspects a session created before ledger markers existed
- **THEN** known current records are shown, but historical totals remain labelled incomplete even if the earlier turn count was zero, and no priced term is named as unapplied for that reason alone.

#### Scenario: Price and usage gaps
- **WHEN** a report lacks cached tokens, terminal usage or a verified price component
- **THEN** the corresponding component or estimate is unknown/incomplete rather than zero; a later catalog price change does not revise the stored estimate basis.

#### Scenario: Unapplied priced term is named
- **WHEN** a schedule prices cache writes, or a long-context tier cannot be decided because the input count was never reported
- **THEN** the estimate stays incomplete, names that term, applies no part of an undecided tier, and keeps the record's raw components and frozen price for a later reading.

### Requirement: Effective-request context fit

Before each inference dispatch, Kuru SHALL estimate the selected effective connector request after native pending continuation and current receipts are assembled. For a route/model with a sourced supported tokenizer and fully represented text, Kuru SHALL derive a labelled local tokenizer estimate from the final serialized request with a documented allowance for provider structure. When that route has saved native output, Kuru SHALL estimate every retained native output-item range across successive pending hops with the labelled byte heuristic while tokenizing the remaining final serialized body, without changing the transmitted request. Unknown/custom routes and unsupported mappings SHALL retain a visibly labelled historical byte heuristic plus the structural allowance. Kuru MUST NOT describe any estimate as an exact provider token count or a guaranteed upper bound. Instructions, tool schemas, current user input, required receipt/call chain and native pending bytes SHALL be mandatory. It MAY omit only older optional history rows as whole units, without mutating durable history, and SHALL report actual omitted row counts and source labels. If mandatory material plus output reserve exceeds the selected model window, it MUST refuse before network dispatch. Unknown model windows SHALL use a documented, visibly labelled conservative assumption rather than block inference solely for missing metadata. Byte transport limits remain independently enforced. Normal inference SHALL NOT call a remote token-count endpoint to decide fit.

#### Scenario: Oversized native continuation
- **WHEN** encrypted pending continuation and matching tool results exceed a small configured window even though visible messages alone appear to fit
- **THEN** the connector refuses before HTTP, preserves the actor-private continuation, and neither tool results nor opaque bytes are silently dropped.

#### Scenario: Optional history is omitted
- **WHEN** old visible history must be reduced to fit a request
- **THEN** complete older rows are omitted only from that request, current receipts remain intact, persisted rows remain unchanged, and the reported omission count equals the rows actually omitted.

#### Scenario: Sourced tokenizer request
- **WHEN** the final selected text request uses a supported route and catalog model with a verified encoding mapping
- **THEN** the preflight uses the pinned tokenizer-derived estimate and structural allowance on that final request, labels its provenance and checks it with the output reserve before dispatch.

#### Scenario: Unsupported or opaque request
- **WHEN** the selected request uses an unmapped model, custom Responses base or opaque pending output items
- **THEN** the preflight labels its whole-body byte fallback or mapped-route mixed native estimate respectively, without deleting transmitted pending bytes, changing model choice or adding a remote counting call.
