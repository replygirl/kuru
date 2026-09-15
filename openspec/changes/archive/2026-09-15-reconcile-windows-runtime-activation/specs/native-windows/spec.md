## MODIFIED Requirements

### Requirement: Durable Windows memory transitions

Windows SHALL preserve lossless SQLite backup migration, complete isolated dream
candidates, expected-base promotion, compensating undo, accepted-operation
reconciliation and staged activation recovery. Publication MUST use validated
same-volume durable operations under the stable lifecycle guard. It MUST NOT
use cross-volume copy/delete or silently omit required durability steps.
An error after a possible native move MUST be reconciled against held identity
and recorded receipts before a retry or a claimed publication outcome. When a
bundled-runtime activation move returns access denied and fresh checked
observations prove that the source retains the held identity and the destination
is absent, Windows activation SHALL retry for no more than two seconds while
retaining the same private stage, source authority and cache lock. Any other
error or observed state MUST fail immediately without retrying publication.

#### Scenario: SQLite WAL migration through native paths
- **WHEN** a legacy fixture with accepted WAL content is migrated from a path containing spaces and Unicode
- **THEN** all accepted content and stable identifiers survive in Dolt, the source remains available, and a failed activation can recover without a partial live database.

#### Scenario: Dream is cancelled or promotion conflicts
- **WHEN** a whole-dream candidate is cancelled or its expected base has advanced
- **THEN** live memory remains unchanged, the candidate follows existing recovery policy, and later conversation and preference writes are retained.

#### Scenario: Activation is interrupted
- **WHEN** the process is interrupted at a staged rename or marker publication boundary
- **THEN** reopening under the stable lifecycle guard recovers a valid recorded state without treating invalid or partial storage as an empty project.

#### Scenario: Publication returns an uncertain outcome
- **WHEN** a native operation reports an error after its move may have occurred
- **THEN** recovery checks actual identity and receipts before retrying or reporting success, preserving recoverable old bytes without assuming the destination stayed unchanged.

#### Scenario: Runtime activation is proven not to have moved
- **WHEN** Windows reports access denied while activating a verified bundled runtime and checked observations show the held source identity remains at its original name while the destination name is absent
- **THEN** Kuru retries the same activation only within the bounded two-second recovery period while retaining the private stage and cache lock, and success still requires the destination to acquire the verified source identity.

#### Scenario: Runtime activation outcome is not proven safe to retry
- **WHEN** activation reports another error, the source identity changes or cannot be observed, or the destination is occupied or cannot be observed
- **THEN** Kuru performs no activation retry, preserves the original typed error and recoverable stage, and does not report publication success.
