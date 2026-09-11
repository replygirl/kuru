# embedded-runtime Specification

## Purpose
Make every Kuru executable self-contained for persistent offline memory by
embedding its verified target-specific full-Dolt archive and licenses, with
explicit build preparation and local-only runtime provisioning.

## Requirements

### Requirement: Complete executable runtime

Every ordinary Kuru executable SHALL contain its target-specific pinned full-Dolt
archive and upstream licenses. A fresh installed executable MUST initialize and
persist memory without a separately installed Dolt or runtime engine download.

#### Scenario: Offline first conversation
- **WHEN** only Kuru is installed with empty memory and engine cache and no network
- **THEN** a demo conversation persists and remains visible after reopening
- **AND** the matching verified engine and licenses are extracted locally

#### Scenario: Installation and update preserve completeness
- **WHEN** Kuru is installed through direct, mise or source installation or updated
- **THEN** the resulting executable carries the engine required for an offline
  first launch with an empty extraction cache

### Requirement: Explicit verified build inputs

Memory SHALL own a native mise preparation task for immutable pinned build inputs.
Cargo MUST select the archive for TARGET, validate local bytes and fail missing or
invalid inputs without downloading or producing an unbundled executable.

#### Scenario: Offline prepared build
- **WHEN** a valid target archive is imported or reused from an offline mirror
- **THEN** the build embeds exactly those pinned bytes including their licenses

#### Scenario: Invalid or mismatched input
- **WHEN** the selected target archive is absent, truncated, corrupt or unsafe
- **THEN** preparation or compilation fails explicitly without a host fallback

### Requirement: Safe local extraction

Runtime provisioning MUST retain bounded archive and payload validation, private
staging, stable locking, cancellation cleanup, exact-version checks and atomic
activation. It SHALL have no runtime HTTP engine-download path.

#### Scenario: Corrupt cache or archive
- **WHEN** an embedded archive or existing cache fails integrity validation
- **THEN** Kuru executes no unverified payload and preserves existing installed data

#### Scenario: Concurrent or cancelled first use
- **WHEN** extraction callers race or a caller is cancelled
- **THEN** staging remains owned until work stops and another caller observes only
  a fully verified activated runtime
