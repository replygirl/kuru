## ADDED Requirements

### Requirement: Bounded built-in shell operational diagnostics

When an authorized built-in shell cannot complete its normal capture and
cleanup flow, the connector SHALL return one finite Kuru-authored operational
category and a bounded recognizable-secret-projected stderr diagnostic on both
Unix and Windows.  The public diagnostic MUST contain no command, arguments,
stdout, native process observation, raw error/cleanup text, or source chain.
It MUST expose an EOF-complete stderr stream only after streaming projection and
the existing 4 KiB diagnostic bound; a stream that has not reached EOF MUST be
rendered as the fixed explicit pending-EOF state and MUST NOT expose its partial
bytes.  The outward error MUST retain no raw source-chain bypass under Display,
alternate Display, or Debug formatting.

A child whose root status and both output streams have completed successfully
MUST retain the existing structured shell JSON result, including its original
nonzero exit status, stdout, and stderr.  Shell operational diagnostics MUST
NOT change Unix retained-owner admission, process-group cleanup/reap authority,
or Windows process/pipe ownership.

#### Scenario: Incomplete stderr after timeout

- **WHEN** an authorized built-in shell times out while its stderr pipe remains open after emitting a fake recognizable credential
- **THEN** its public error has the fixed timeout category and pending-EOF state, and no rendering or source-chain element contains the command, credential, partial stderr, stdout, or raw cleanup detail.

#### Scenario: Complete stderr before operational failure

- **WHEN** an authorized built-in shell reaches stderr EOF containing a fake recognizable credential before a later operational failure
- **THEN** its public error contains the same cross-platform category grammar and a 4 KiB-bounded redacted stderr excerpt with no raw matched credential.

#### Scenario: Completed nonzero shell exit

- **WHEN** an authorized built-in shell exits nonzero after both output streams reach EOF and cleanup is confirmed
- **THEN** it returns the existing structured JSON result with `success` false and its original exit code rather than an operational diagnostic.
