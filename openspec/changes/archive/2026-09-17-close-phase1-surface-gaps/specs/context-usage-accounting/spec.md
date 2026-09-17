## MODIFIED Requirements

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
