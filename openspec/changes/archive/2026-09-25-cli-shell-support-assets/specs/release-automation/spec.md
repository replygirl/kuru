## ADDED Requirements

### Requirement: Complete immutable shell-support release inventory

Release candidate assembly SHALL require one versioned support envelope paired with each existing native executable archive, with exactly one checksum-manifest entry per sidecar and no unexpected assets. It SHALL verify all five decoded support inventories and each sidecar's generated content against its paired target build before final publication. The staged and published-download verifiers SHALL check the exact immutable support assets selected for the release without weakening the existing executable archive member inventory or native offline-runtime gates.

#### Scenario: Incomplete candidate
- **WHEN** any target's paired support envelope, checksum entry or expected generated file is missing or inconsistent with that target's executable
- **THEN** candidate assembly fails before the release can be published.

#### Scenario: Published asset differs
- **WHEN** a published support envelope differs from the release's immutable checksum or exact member inventory
- **THEN** post-publication verification reports that failure without mutating or unpublishing the release.
