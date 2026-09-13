# release-automation Specification

## Purpose
Prepare and publish reproducible Kuru releases from reviewed main history using
manual dispatch, conventional versions, signed commits, complete verified
artifacts, and generated release notes with recoverable publication steps.

## Requirements

### Requirement: Deliberate conventional version preparation

The release workflow SHALL run only through manual dispatch with one auto, major, minor or patch strategy input. It SHALL calculate versions from conventional history without modifying the repository during calculation and automatically recognize an existing release candidate at the selected commit.

#### Scenario: Initial release already matches the manifest
- **WHEN** computed version equals the workspace version and no release exists
- **THEN** preparation uses the unchanged validated commit without fabricating a version change

#### Scenario: Version calculation does not return a version
- **WHEN** Cocogitto returns a no-op message or invalid version for new history
- **THEN** the workflow fails before remote mutation

#### Scenario: Rerun after a version commit or tag exists
- **WHEN** the original release run is retried
- **THEN** preparation preserves its original source and selected version without requiring resume inputs

### Requirement: Immutable validated release identity

The workflow SHALL create signed version commits using an expected main head, validate the exact candidate commit, and bind every archive and deployment to that commit. Recovery SHALL recognize an existing direct version commit only when its parent, release message and full tree match the expected stamp.

#### Scenario: Main changes during preparation
- **WHEN** unrelated history moves main before the API commit
- **THEN** the expected-head comparison rejects mutation and the release stops

#### Scenario: Lost version commit response
- **WHEN** the API accepted the exact version commit but the runner failed before recording its SHA
- **THEN** retry reuses that commit without another mutation, even if later unrelated commits advanced main

#### Scenario: Conflicting existing tag
- **WHEN** the selected tag resolves to another commit
- **THEN** publication fails without deleting or changing the existing tag

### Requirement: Complete recoverable publication

The workflow SHALL verify all four expected archives and checksums, generate notes before publication, stage a complete draft, and publish only after successful validation. Reruns SHALL reuse a complete matching published release without replacing its notes, tag or assets.

#### Scenario: Incomplete or corrupted artifacts
- **WHEN** an expected archive is missing, unexpected, or checksum-invalid before publication
- **THEN** no release is published

#### Scenario: Retry after partial preparation
- **WHEN** a maintainer reruns an interrupted release job
- **THEN** the workflow reuses that identity and safely completes missing publication steps without replacing an existing published release

#### Scenario: Publication succeeded before the runner failed
- **WHEN** retry finds a published release with the exact immutable tag, source marker and complete valid asset metadata
- **THEN** publication succeeds without remote writes and the final documentation jobs can proceed

### Requirement: Accurate release notes and instructions

Release notes SHALL use supported Communiqué configuration and include initial implementation context for the first release. Documentation SHALL describe required release credentials and direct binary installation, including explicit versions and local release directories, without repository-visibility commentary.

#### Scenario: Initial commit contains the harness
- **WHEN** the first release has no previous version tag
- **THEN** notes generation receives the root commit inventory in addition to later conventional history

#### Scenario: Installation from a downloaded release
- **WHEN** a user has the matching native archive and checksum manifest in a local directory
- **THEN** the documented installer can use that directory and explicit version without a compiler or a network request

### Requirement: Published Windows release gate

The Release workflow SHALL verify the exact published Windows release from the
selected version commit after publication and before building or deploying
documentation. The gate MUST be rerunnable in the same release run without
changing the release, tag, assets, or selected commit.

#### Scenario: Published Windows verification succeeds
- **WHEN** the exact published tag, commit, assets, installation, bundled runtime, and persistent offline behavior pass on native Windows
- **THEN** the same-run documentation build and deployment may proceed

#### Scenario: Published Windows verification fails
- **WHEN** any required published-release observation fails or cleanup is not confirmed
- **THEN** the verification job fails and the dependent documentation jobs do not run, without modifying the published release

#### Scenario: Published release run resumes
- **WHEN** a maintainer reruns the verification job after publication already succeeded
- **THEN** it verifies the existing immutable release selected by the original run without dispatching a new release
