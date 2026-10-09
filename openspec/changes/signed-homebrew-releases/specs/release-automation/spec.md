## ADDED Requirements

### Requirement: Signed candidate reuse

Release automation SHALL finish native publisher signing before final archive and
checksum creation, retain each signed native package with its selected source
commit, version and target, and reuse that exact validated package on a retry of
the same release run. It MUST NOT replace accepted artifacts with newly timestamped
signatures or mutate public assets. The existing single manual workflow and its
staged native and documentation gates SHALL remain authoritative.

#### Scenario: Rerun all jobs after signing
- **WHEN** a release run already has a matching retained signed target package
- **THEN** the build/signing path restores and validates it instead of signing another copy.

#### Scenario: Retained package has a conflicting identity
- **WHEN** its source commit, target, version, inventory or digest differs
- **THEN** retry fails before candidate assembly or publication.

### Requirement: Post-publication tap recovery

Tap publication SHALL follow successful release promotion and remain recoverable
by rerunning the original release workflow. A tap failure SHALL report failure
without changing or unpublishing the release. Recovery of an already public
release SHALL use its public manifest rather than newly produced local artifacts.

#### Scenario: Tap update fails after release publication
- **WHEN** the release is public but a tap update fails
- **THEN** retry validates the immutable public release and completes only the missing tap update.

## MODIFIED Requirements

### Requirement: Complete recoverable publication

The workflow SHALL verify one expected native archive and checksum sidecar for every target in the authoritative release catalog, generate the complete `SHA256SUMS` manifest and release notes, and assemble them into one attempt-scoped candidate artifact without making a remote release write. It SHALL publish only from the sole promotion job after native acceptance of every target, native Homebrew acceptance on its supported targets, and documentation build and deployment have succeeded. Final publication SHALL revalidate the candidate, preserve complete-draft digest checks, and reuse a complete matching published release without replacing its notes, tag, or assets. Post-publication checks and tap updates SHALL retain their separate failure and recovery outcomes.

#### Scenario: Incomplete or corrupted artifacts
- **WHEN** an expected archive, checksum sidecar, checksum-manifest entry, or notes file is missing, unexpected, malformed, or invalid
- **THEN** candidate assembly or final revalidation fails and no public release is created.

#### Scenario: Required staged acceptance fails
- **WHEN** native staged acceptance, Homebrew acceptance, documentation build, or documentation deployment fails or is interrupted
- **THEN** the promotion job does not run and no draft is promoted to a public release.

#### Scenario: Retry after partial preparation
- **WHEN** a maintainer reruns an interrupted release workflow after a candidate or private draft was prepared
- **THEN** the workflow preserves the selected identity, reuses only matching validated state, and completes missing publication steps without replacing an existing published release.

#### Scenario: Publication succeeded before the runner failed
- **WHEN** retry finds a published release with the exact immutable tag, source marker, and complete valid asset metadata
- **THEN** the promotion job succeeds without remote writes and post-publication verification or tap updates can resume.

#### Scenario: Retired target archive in the candidate
- **WHEN** a candidate contains an archive or checksum sidecar for a target that is not in the release catalog, such as `x86_64-apple-darwin`
- **THEN** candidate assembly rejects it as an unexpected asset and no public release is created.
