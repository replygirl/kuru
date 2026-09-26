## MODIFIED Requirements

### Requirement: Complete recoverable publication

The workflow SHALL verify one expected native archive and checksum sidecar for every target in the authoritative release catalog, generate the complete `SHA256SUMS` manifest and release notes, and assemble them into one attempt-scoped candidate artifact without making a remote release write. It SHALL publish only from the sole final job after native Windows acceptance of that candidate and documentation build and deployment have succeeded. Final publication SHALL revalidate the candidate, preserve complete-draft digest checks, and reuse a complete matching published release without replacing its notes, tag, or assets.

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

#### Scenario: Retired target archive in the candidate
- **WHEN** a candidate contains an archive or checksum sidecar for a target that is not in the release catalog, such as `x86_64-apple-darwin`
- **THEN** candidate assembly rejects it as an unexpected asset and no public release is created

### Requirement: Complete immutable shell-support release inventory

Release candidate assembly SHALL require one versioned support envelope paired with each existing native executable archive, with exactly one checksum-manifest entry per sidecar and no unexpected assets. It SHALL verify one core archive and one paired decoded support inventory for every target in the release catalog, and each sidecar's generated content against its paired target build, before final publication. The staged and published-download verifiers SHALL check the exact immutable support assets selected for the release without weakening the existing executable archive member inventory or native offline-runtime gates.

#### Scenario: Incomplete candidate
- **WHEN** any target's paired support envelope, checksum entry or expected generated file is missing or inconsistent with that target's executable
- **THEN** candidate assembly fails before the release can be published.

#### Scenario: Published asset differs
- **WHEN** a published support envelope differs from the release's immutable checksum or exact member inventory
- **THEN** post-publication verification reports that failure without mutating or unpublishing the release.
