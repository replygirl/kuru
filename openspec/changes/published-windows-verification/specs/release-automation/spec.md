## ADDED Requirements

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
- **THEN** the verification job and dependent documentation jobs fail without modifying the published release

#### Scenario: Published release run resumes
- **WHEN** a maintainer reruns the verification job after publication already succeeded
- **THEN** it verifies the existing immutable release selected by the original run without dispatching a new release
