## ADDED Requirements

### Requirement: Compaction invocation fit and accounting

Every automatic or manual compaction provider request SHALL use a distinct typed compaction phase and the ordinary durable invocation ledger before dispatch. Its stable identity MUST bind the real operation together with the exact actor, session, view, revision and source range, so an exact operation retry cannot duplicate dispatch while a later operation can retry an unchanged source after a terminal failure or cancellation. Its input estimate MUST cover the exact instruction, prior cursor-selected summary, selected whole source rows and structural allowance; its output reserve MUST be the configured compaction reserve rather than the later ordinary request reserve. Reported usage, cancellation, failure, incomplete evidence, frozen price and accepted lost-reply recovery SHALL follow the same accounting contract as other provider invocations. The estimate itself MUST be pure and MUST NOT make an extra provider request.

#### Scenario: Summary request reports usage then fails
- **WHEN** a compaction request reports partial usage and then fails or is cancelled
- **THEN** its stable actor/session/source-range invocation retains that partial evidence as incomplete, the summary cursor does not advance, and retry does not rewrite the usage as an ordinary speaking call.

#### Scenario: Compaction input cannot fit
- **WHEN** the instruction, prior summary, one complete next source row and compaction output reserve exceed the effective model window
- **THEN** Kuru refuses before provider dispatch, records no invented usage and leaves the existing summary and cursor unchanged.
