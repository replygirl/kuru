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

The repository SHALL document direct source and mise-managed source installation that work before public releases exist.

#### Scenario: Source installation
- **WHEN** the source installer runs with an explicit writable destination
- **THEN** the resulting executable reports its version and runs the offline demo.

### Requirement: Verified release installation and update

Release packaging SHALL produce platform archives and SHA-256 checksums. Release installation and updates MUST verify checksums and expected archive paths before atomic executable replacement, using an explicitly configured release source until publication metadata exists.

#### Scenario: Corrupted download
- **WHEN** a candidate update does not match the checksum manifest
- **THEN** installation fails and the previous executable remains unchanged.

### Requirement: Accurate operational documentation

Documentation SHALL describe cognition scope, commands, storage, permissions, protocols, authentication, install/update and development workflow, and distinguish verified local behavior from external release/service dependencies.

#### Scenario: Unpublished release
- **WHEN** no public release repository is configured
- **THEN** installation instructions use working source paths and do not claim a public binary endpoint exists.
