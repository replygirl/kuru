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

The workflow SHALL verify all five expected native archives and checksum sidecars, generate the complete `SHA256SUMS` manifest and release notes, and assemble them into one attempt-scoped candidate artifact without making a remote release write. It SHALL publish only from the sole final job after native Windows acceptance of that candidate and documentation build and deployment have succeeded. Final publication SHALL revalidate the candidate, preserve complete-draft digest checks, and reuse a complete matching published release without replacing its notes, tag, or assets.

#### Scenario: Incomplete or corrupted artifacts
- **WHEN** an expected archive, checksum sidecar, checksum-manifest entry, or notes file is missing, unexpected, malformed, or invalid
- **THEN** candidate assembly or final revalidation fails and no public release is created

#### Scenario: Required staged acceptance fails
- **WHEN** staged Windows acceptance, documentation build, or documentation deployment fails or is interrupted
- **THEN** the final publication job does not run and no draft is promoted to a public release

#### Scenario: Retry after partial preparation
- **WHEN** a maintainer reruns an interrupted release workflow after a candidate or private draft was prepared
- **THEN** the workflow preserves the selected identity, reuses only matching validated state, and completes missing publication steps without replacing an existing published release

#### Scenario: Publication succeeded before the runner failed
- **WHEN** retry finds a published release with the exact immutable tag, source marker, and complete valid asset metadata
- **THEN** the final publication job succeeds without remote writes and no dependent release jobs remain

### Requirement: Accurate release notes and instructions

Release notes SHALL use supported Communiqué configuration and include initial implementation context for the first release. Documentation SHALL describe required release credentials and direct binary installation, including explicit versions and local release directories, without repository-visibility commentary.

#### Scenario: Initial commit contains the harness
- **WHEN** the first release has no previous version tag
- **THEN** notes generation receives the root commit inventory in addition to later conventional history

#### Scenario: Installation from a downloaded release
- **WHEN** a user has the matching native archive and checksum manifest in a local directory
- **THEN** the documented installer can use that directory and explicit version without a compiler or a network request

### Requirement: Staged Windows release gate

The Release workflow SHALL verify the exact staged Windows ZIP from the selected version commit through native mise before documentation deployment and public release promotion. The staged gate MUST use the complete candidate artifact selected for final publication, remain bounded and isolated, and MUST NOT be represented as verification of an actual public download.

#### Scenario: Staged Windows verification succeeds
- **WHEN** native mise selects the simulated exact release metadata, downloads the staged candidate ZIP, verifies its checksum, installs it, and its bundled runtime and persistent offline behavior pass
- **THEN** documentation deployment may proceed while final publication remains gated on every other required result

#### Scenario: Staged Windows verification fails
- **WHEN** candidate identity, checksum, installation, runtime, persistence, or owned cleanup fails
- **THEN** documentation deployment and public release promotion do not run

#### Scenario: Staged release run resumes
- **WHEN** a maintainer reruns an interrupted release workflow before publication
- **THEN** the rerun remains bound to the original selected commit and a complete validated candidate without requiring an existing public release
