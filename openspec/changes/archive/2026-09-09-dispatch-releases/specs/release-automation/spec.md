## ADDED Requirements

### Requirement: Deliberate conventional version preparation

The release workflow SHALL run only through manual dispatch and SHALL calculate auto, major, minor or patch versions from conventional history without modifying the repository during calculation.

#### Scenario: Initial release already matches the manifest
- **WHEN** computed version equals the workspace version and no release exists
- **THEN** preparation uses the unchanged validated commit without fabricating a version change

#### Scenario: Version calculation does not return a version
- **WHEN** Cocogitto returns a no-op message or invalid version
- **THEN** the workflow fails before remote mutation

### Requirement: Immutable validated release identity

The workflow SHALL create signed version commits using an expected main head, validate the exact candidate commit, and bind every archive and deployment to that commit.

#### Scenario: Main changes during preparation
- **WHEN** another commit moves main before the API commit
- **THEN** the expected-head comparison rejects mutation and the release stops

#### Scenario: Conflicting existing tag
- **WHEN** the selected tag resolves to another commit
- **THEN** publication fails without deleting or changing the existing tag

### Requirement: Complete recoverable publication

The workflow SHALL verify all four expected archives and checksums, generate notes before publication, stage a complete draft, and publish only after successful validation.

#### Scenario: Incomplete or corrupted artifacts
- **WHEN** an expected archive is missing, unexpected, or checksum-invalid
- **THEN** no release is published

#### Scenario: Retry after partial preparation
- **WHEN** a maintainer explicitly resumes an unpublished version at its original commit
- **THEN** the workflow reuses that identity and safely completes missing publication steps without replacing an existing published release

### Requirement: Accurate release notes and instructions

Release notes SHALL use supported Communiqué configuration and include initial implementation context for the first release. Documentation SHALL describe required credentials and preserve authenticated installation while the repository is private.

#### Scenario: Initial commit contains the harness
- **WHEN** the first release has no previous version tag
- **THEN** notes generation receives the root commit inventory in addition to later conventional history
