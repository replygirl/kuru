# homebrew-distribution Specification

## Purpose

Distribute complete native Kuru releases through the dedicated Homebrew tap on
supported macOS and Linux targets. Formulas preserve verified application and
support payloads, upgrades preserve existing conversations, and tap updates remain
bound to the immutable public release.

## Requirements

### Requirement: Exact binary formula inventory

The Kuru Homebrew formula SHALL select the supported macOS ARM64 and Linux
x86_64/ARM64 native release archives and their paired shell-support resources from
one complete verified release manifest. It SHALL install the unchanged executable,
embedded engine, licenses, manual and four completions without an external runtime
engine, compiler or source build. Unsupported targets MUST fail explicitly.

#### Scenario: Formula generation has an incomplete manifest
- **WHEN** any required native archive or support resource digest is absent or malformed
- **THEN** formula generation fails instead of emitting a partially installable formula.

#### Scenario: Homebrew changes binary linkage
- **WHEN** native Homebrew installation rewrites a signed executable
- **THEN** installed digest acceptance fails and publication remains blocked.

### Requirement: Native Homebrew installation acceptance

Before release promotion, supported native release legs SHALL exercise Homebrew
installation, formula tests, exact installed payload comparison, an offline cold
conversation and reopen, owned cleanup, and previous-release-to-candidate upgrade
when a previous release exists. Homebrew-owned executables SHALL refuse Kuru
self-update before network or staging effects.

#### Scenario: Upgrade an existing release
- **WHEN** Homebrew replaces the previous release with the candidate
- **THEN** the installed version and payload match the candidate and offline persistent use succeeds.

### Requirement: Idempotent scoped tap publication

After public release promotion, the Release workflow SHALL update
`replygirl/homebrew-kuru` using a separately scoped GitHub App. Publication SHALL
derive formula bytes from the actual immutable public release identity and asset
manifest, use compare-and-swap file updates, reject version rollback and conflicting
same-version bytes, and reuse identical formula content without a write. Tap
updates MUST NOT dispatch releases or broaden the source repository's release app.

#### Scenario: Tap already has this exact formula
- **WHEN** a retry finds identical formula bytes
- **THEN** it succeeds without creating another commit.

#### Scenario: Concurrent or newer tap update
- **WHEN** the tap changes after inspection or already contains a newer version
- **THEN** publication fails without overwriting that state.
