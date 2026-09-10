## MODIFIED Requirements

### Requirement: Reproducible workspace quality gates

The repository SHALL use apps/ and packages/, pinned Rust/mise tooling, hk hooks,
cospec change gates, Cargo.lock, and CI checks for format, lint, tests and at
least 90% workspace line coverage from meaningful behavioral tests. Native
Windows x64/MSVC verification SHALL be required by the aggregate CI gate.
Windows-only behavior MUST be executed and measured on Windows, not inferred
from Unix checks or compiled-out tests.

#### Scenario: Coverage regression
- **WHEN** measured workspace line coverage is below 90 percent
- **THEN** the coverage check fails rather than silently reducing the threshold or excluding application code.

#### Scenario: Native Windows gate fails
- **WHEN** the Windows job has a failing required test or coverage check
- **THEN** the aggregate gate fails regardless of successful macOS and Linux jobs.

### Requirement: Source and mise installation

The repository SHALL document mise GitHub binary installation and compiler-free
platform shell installation before source installation. Windows instructions
SHALL use native PowerShell and its matching executable archive. Source
installation SHALL use the pinned Rust toolchain through prepared app mise
tasks, include the target's embedded runtime and support an explicit destination.
Mise instructions SHALL distinguish installation from activation and explain
exact version selection.

#### Scenario: Direct mise installation
- **WHEN** a user selects a published Kuru version through mise's GitHub backend
- **THEN** mise installs the matching native executable without a checkout or compiler and can activate it for invocation.

#### Scenario: Source installation
- **WHEN** the source installer runs with an explicit writable destination
- **THEN** the resulting executable reports its version and runs the offline demo with an initially empty runtime cache.

#### Scenario: Native PowerShell installation
- **WHEN** the Windows bootstrap runs in stock PowerShell with a supported published version
- **THEN** it installs the verified native executable without Bash, a compiler, a checkout or a separate Dolt installation.

### Requirement: Verified release installation and update

Release packaging SHALL produce platform archives and SHA-256 checksums.
Release installation and updates MUST verify checksums and expected archive
paths before replacement. Ordinary executable replacement SHALL be atomic and
durable on the supported native filesystem. Platform shell bootstraps SHALL
resolve latest once to an explicit archive version or accept an explicit
version, bound downloads and decompression, and extract only the single regular
executable into private staging. Failures and handled interruptions before
replacement MUST preserve an existing executable and remove temporary staging.
The native Rust updater SHALL retain its full archive validation and
explicit-version behavior.

Windows running-executable updates SHALL preserve verified old bytes and record
any multi-step replacement transition for recovery. Handled publication failure
MUST restore the old executable path. Success MUST mean the installed path
contains the validated replacement, even when cleanup of a displaced loaded
image finishes after process exit. Multi-step replacement MUST NOT be described
as a single atomic swap. Validation and cleanup MUST NOT execute a downloaded
candidate. No reboot or elevated permission SHALL be required for installation
in a writable user directory.

#### Scenario: Corrupted download
- **WHEN** a candidate update does not match the checksum manifest
- **THEN** installation fails and the previous executable remains unchanged.

#### Scenario: Latest changes during download
- **WHEN** latest metadata selects a version and the repository publishes another version before archive download
- **THEN** the bootstrap downloads the already-selected explicit version and verifies its selected checksum.

#### Scenario: Unsafe archive or destination
- **WHEN** an archive contains unexpected, duplicate or linked entries, or the destination executable is a symlink, reparse point or directory
- **THEN** the bootstrap rejects it without broad archive extraction or replacement of the destination.

#### Scenario: Running Windows executable updates itself
- **WHEN** the real installed Windows executable requests a verified update in its writable directory
- **THEN** the installed path contains the verified new executable before success is reported, the previous loaded image is cleaned up safely, and no candidate was executed to perform validation or cleanup.

#### Scenario: Windows replacement is interrupted
- **WHEN** a fixture interrupts a multi-step loaded-image replacement at a recorded publication boundary
- **THEN** recovery preserves a valid old or fully verified new installation and does not silently lose the original bytes or execute an unverified candidate.

### Requirement: Accurate operational documentation

Documentation SHALL describe cognition scope, commands, storage, permissions,
protocols, authentication, install/update and development workflow. User
installation instructions SHALL directly describe available commands without
repository-visibility or future-readiness commentary. Native Windows defaults,
PowerShell commands, supported filesystem guarantees and running-executable
update behavior SHALL match the implementation. Verification records SHALL
distinguish measured local behavior from observed external publication and
installation results.

#### Scenario: Unpublished release
- **WHEN** a requested release version has not been published
- **THEN** installation reports the download failure and preserves an installed executable rather than silently choosing another version.

#### Scenario: Installation choices
- **WHEN** a user reads the README installation section
- **THEN** mise and native platform shell binary installation appear before source installation, with working command syntax and links to detailed platform and update instructions.

## ADDED Requirements

### Requirement: Native Windows release artifact

The release workflow SHALL build `x86_64-pc-windows-msvc` from the same prepared
version commit as the existing four native targets and publish its ZIP and
checksum. The ZIP SHALL contain exactly the flat regular members `kuru.exe`,
`LICENSE` and `README.md`; the executable SHALL include its verified full Dolt
bundle. Target selection, expected executable names and formats SHALL have one
authoritative catalog. The release SHALL retain strategy-only dispatch,
conventional-commit version selection, automatic exact-commit recovery,
immutable publication and final Pages jobs after successful publication.

#### Scenario: Windows artifact is missing or invalid
- **WHEN** any required Windows package or native packaged-runtime check fails
- **THEN** publication does not create a partial successful release or run the final Pages deployment.

#### Scenario: Release run resumes
- **WHEN** an interrupted release is rerun after some target artifacts were prepared
- **THEN** recovery uses the existing planned version commit, verifies the full five-target inventory and does not create another bump or overwrite published assets.
