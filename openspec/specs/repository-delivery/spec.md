# repository-delivery Specification

## Purpose
Keep the Rust monorepo reproducible through pinned tools and dependencies,
cospec change artifacts, meaningful coverage and automated quality gates.
Provide documented source installation and verified, atomic release updates.

## Requirements

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

#### Scenario: Candidate mise installation before publication
- **WHEN** native acceptance routes the real mise GitHub backend through supported URL replacements to simulated release metadata and the exact staged Windows ZIP selected for final publication
- **THEN** native selection, checksum enforcement, installation, activation, and persistent offline runtime behavior are verified without live fallback or a public download, while the package-owned published verifier remains available as a separate post-publication diagnostic.

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

### Requirement: Package-owned memory runtime verification

The memory package SHALL own runtime provisioning and real Dolt integration
fixtures through native mise tasks. Required tests MUST fail when Dolt cannot be
provisioned or started. CI SHALL exercise the supported native release platforms;
the existing workspace coverage threshold and release archive contract SHALL remain.

#### Scenario: Missing test runtime
- **WHEN** a required integration check cannot obtain its pinned Dolt executable
- **THEN** the check fails with actionable diagnostics instead of skipping or substituting SQLite.

### Requirement: Native Windows release artifact

The release workflow SHALL build `x86_64-pc-windows-msvc` from the same prepared
version commit as the other native targets in the release catalog and include
its ZIP and checksum in the complete staged candidate. The ZIP SHALL contain
exactly the flat regular members `kuru.exe`, `LICENSE` and `README.md`; the
executable SHALL include its verified full Dolt bundle. Target selection,
expected executable names and formats SHALL have one authoritative catalog. The
release SHALL retain strategy-only dispatch, conventional-commit version
selection, automatic exact-commit recovery, immutable publication, and
documentation deployment from the selected commit before the sole final
public-release job.

#### Scenario: Windows artifact is missing or invalid
- **WHEN** any required Windows package or native packaged-runtime check fails
- **THEN** candidate acceptance fails, final Pages deployment does not run, and no partial successful release is made public.

#### Scenario: Release run resumes
- **WHEN** an interrupted release is rerun after some target artifacts were prepared
- **THEN** recovery uses the existing planned version commit, verifies one core and one paired support archive for every catalog target in the candidate inventory and does not create another bump or overwrite published assets.

### Requirement: Package-owned published Windows verifier

The delivery package SHALL provide one native Windows verifier and mise task for
an exact published Kuru version and expected commit. The verifier MUST use the
ordinary `github:replygirl/kuru@VERSION` backend in isolated mise and user roots,
with no inherited release token, provider credential, proxy, OAuth store, lock,
or endpoint replacement. It MUST independently verify the published tag and
commit, complete release asset inventory, checksum manifest, Windows archive,
selected installed executable, embedded Dolt executable, and bundled licenses.

#### Scenario: Ordinary published installation
- **WHEN** the verifier runs with an exact published version, expected commit, checked-out asset manifest, and native mise executable
- **THEN** it selects, installs, locates, and executes that version through mise without a custom endpoint, a compiler-built Kuru executable, or a separately installed Dolt

#### Scenario: Publication identity disagrees
- **WHEN** the tag, resolved commit, asset inventory, checksums, archive, installed executable, engine, or licenses disagree with the exact expected release
- **THEN** verification fails rather than accepting mise installation alone as provenance evidence

### Requirement: Bounded published verification receipt

Published Windows verification SHALL bound command execution and output, await
owned cleanup, and emit a bounded JSON evidence receipt containing fixed safe
metadata only after isolated state has been removed. It MUST parse Kuru machine
results from stdout without requiring stderr to be empty.

#### Scenario: Verification completes
- **WHEN** every release, installation, runtime, persistence, and cleanup check succeeds
- **THEN** the receipt records exact version and commit identity, digests, command milestones, durable session observations, bundled-runtime observations, and confirmed cleanup without raw child output or credentials

#### Scenario: Child emits informational stderr
- **WHEN** an otherwise successful Kuru command emits an informational first-run notice on stderr while retaining its JSON stdout contract
- **THEN** verification accepts the machine result and keeps only bounded failure diagnostics if a later check fails

### Requirement: Verified shell-support installation with legacy core compatibility

Current direct installers and the native updater SHALL verify and stage the selected release's target-paired shell-support envelope within the explicit installation root before changing the installed executable. They SHALL publish only its exact versioned support directory before the binary, then update a stable owned Unix man file by checked regular-file publication only after confirmed executable replacement. They SHALL preserve the existing executable replacement receipt and report failed, uncertain or partial support publication without claiming a complete new installation. They MUST NOT modify shell profiles or unselected filesystem roots. The existing three-member platform executable archive and its strict validation SHALL remain unchanged, so previously published clients can still upgrade the executable. New clients SHALL parse one bounded shell-support-format marker in the core README only after verifying the core archive; a verified unmarked historical README retains the legacy executable-only path, while a marked release requires its selected target's paired sidecar and malformed, duplicate or unsupported markers are rejected. Current release packaging and verification MUST require the supported marker in every new core archive.

#### Scenario: Support validation fails before binary publication
- **WHEN** the selected support envelope is absent where required, corrupt, oversized, or contains an unsafe member
- **THEN** the installer leaves the existing executable and support directories unchanged and reports the failed installation.

#### Scenario: Executable publication fails after support staging
- **WHEN** support files were verified and published but existing executable replacement fails or remains uncertain
- **THEN** the executable outcome follows its existing receipt, only exactly owned unreferenced support files may be cleaned, and the installer never reports a complete new installation.

#### Scenario: Stable man publication fails after binary success
- **WHEN** the new executable is confirmed but checked publication of the selected Unix `share/man/man1/kuru.1` fails
- **THEN** the installer reports a partial result and an explicit retry revalidates the executable and versioned snapshot before repairing that file, without claiming cross-file atomicity.

#### Scenario: A marked new release omits support assets
- **WHEN** a selected core archive has the supported README marker but its checksum manifest omits its target-paired shell-support envelope
- **THEN** the installer rejects the release before changing the installed executable rather than treating it as legacy.

#### Scenario: A new core has an invalid capability marker
- **WHEN** a release candidate's README lacks the required new-format marker, or a verified core has a malformed, duplicated or unsupported marker
- **THEN** release assembly or installation refuses it without weakening the three-member core archive inventory.

#### Scenario: Upgrade from an older strict reader
- **WHEN** an already-published v0.4.1 or v0.4.2 updater selects a new release
- **THEN** its unchanged three-member core archive remains acceptable; missing shell assets are not misrepresented as installed, and a current verified installer or the new binary's pure local generation offers explicit repair.

#### Scenario: New direct installation
- **WHEN** a current Unix or Windows bootstrap installs a release with support envelopes into an explicit destination
- **THEN** the matching verified versioned support files and executable are present, and a cold offline Kuru conversation still works without a separate engine download.

### Requirement: Intel macOS installation refusal

The release catalog SHALL contain `aarch64-apple-darwin`,
`aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu` and
`x86_64-pc-windows-msvc`, and SHALL NOT contain `x86_64-apple-darwin`. The
shell bootstrap MUST refuse an Intel Mac, whether detected from a `Darwin`
`x86_64` host or selected with `--target x86_64-apple-darwin`, before any
network request, with a non-zero exit and the message
`kuru: Intel Macs (x86_64-apple-darwin) are no longer supported; v0.9.0 was the last release supporting them`.
The refusal MUST leave an existing destination executable unchanged. Published
releases up to v0.9.0 SHALL keep their `x86_64-apple-darwin` assets, and the
installation documentation SHALL direct Intel Mac users to the v0.9.0 tag's own
bootstrap with an explicit `--version 0.9.0`.

#### Scenario: Intel Mac host runs the current bootstrap
- **WHEN** the shell bootstrap runs on a host whose `uname` reports `Darwin` and `x86_64`
- **THEN** it exits non-zero with the v0.9.0 refusal message, makes no download request and leaves the destination unchanged

#### Scenario: Intel target is selected explicitly
- **WHEN** the shell bootstrap is invoked with `--target x86_64-apple-darwin` on any host
- **THEN** it exits non-zero with the same refusal message before any download

#### Scenario: Release catalog excludes Intel macOS
- **WHEN** the release catalog is queried for `x86_64-apple-darwin` or for the macOS `x86_64` platform
- **THEN** no target is found, and release assembly expects no Intel macOS archive
