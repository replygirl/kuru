# repository-delivery Specification

## Purpose
Keep the Rust monorepo reproducible through pinned tools and dependencies,
cospec change artifacts, meaningful coverage and automated quality gates.
Provide documented source installation and verified, atomic release updates.

## Requirements

### Requirement: Reproducible workspace quality gates

The repository SHALL use apps/ and packages/, pinned Rust/mise tooling, hk hooks, cospec change gates, Cargo.lock, and CI checks for format, lint, tests and at least 90% workspace line coverage from meaningful behavioral tests.

#### Scenario: Coverage regression
- **WHEN** measured workspace line coverage is below 90 percent
- **THEN** the coverage check fails rather than silently reducing the threshold or excluding application code.

### Requirement: Source and mise installation

The repository SHALL document mise GitHub binary installation and a compiler-free shell installation before source installation. Source installation SHALL continue to use the pinned Rust toolchain and support an explicit destination. Mise instructions SHALL distinguish installation from activation and explain exact version selection.

#### Scenario: Direct mise installation
- **WHEN** a user selects a published Kuru version through mise's GitHub backend
- **THEN** mise installs the matching native executable without a checkout or compiler and can activate it for invocation.

#### Scenario: Source installation
- **WHEN** the source installer runs with an explicit writable destination
- **THEN** the resulting executable reports its version and runs the offline demo.

### Requirement: Verified release installation and update

Release packaging SHALL produce platform archives and SHA-256 checksums. Release installation and updates MUST verify checksums and expected archive paths before atomic executable replacement. The shell bootstrap SHALL resolve latest once to an explicit archive version or accept an explicit version, bound downloads and decompression, and extract only the single regular executable into private staging. Failures and handled interruptions before replacement MUST preserve an existing executable and remove temporary staging. The native Rust updater SHALL retain its full archive validation and explicit-version behavior.

#### Scenario: Corrupted download
- **WHEN** a candidate update does not match the checksum manifest
- **THEN** installation fails and the previous executable remains unchanged.

#### Scenario: Latest changes during download
- **WHEN** latest metadata selects a version and the repository publishes another version before archive download
- **THEN** the bootstrap downloads the already-selected explicit version and verifies its selected checksum.

#### Scenario: Unsafe archive or destination
- **WHEN** an archive contains unexpected, duplicate or linked entries, or the destination executable is a symlink or directory
- **THEN** the bootstrap rejects it without broad archive extraction or replacement of the destination.

### Requirement: Accurate operational documentation

Documentation SHALL describe cognition scope, commands, storage, permissions, protocols, authentication, install/update and development workflow. User installation instructions SHALL directly describe available commands without repository-visibility or future-readiness commentary. Verification records SHALL distinguish measured local behavior from observed external publication and installation results.

#### Scenario: Unpublished release
- **WHEN** a requested release version has not been published
- **THEN** installation reports the download failure and preserves an installed executable rather than silently choosing another version.

#### Scenario: Installation choices
- **WHEN** a user reads the README installation section
- **THEN** mise and shell binary installation appear before source installation, with working command syntax and links to detailed platform and update instructions.

### Requirement: Package-owned memory runtime verification

The memory package SHALL own runtime provisioning and real Dolt integration
fixtures through native mise tasks. Required tests MUST fail when Dolt cannot be
provisioned or started. CI SHALL exercise the supported native release platforms;
the existing workspace coverage threshold and release archive contract SHALL remain.

#### Scenario: Missing test runtime
- **WHEN** a required integration check cannot obtain its pinned Dolt executable
- **THEN** the check fails with actionable diagnostics instead of skipping or substituting SQLite.
