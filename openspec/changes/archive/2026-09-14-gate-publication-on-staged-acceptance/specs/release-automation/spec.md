## MODIFIED Requirements

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

## ADDED Requirements

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

## REMOVED Requirements

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
