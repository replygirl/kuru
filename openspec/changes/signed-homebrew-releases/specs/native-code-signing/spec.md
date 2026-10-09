## ADDED Requirements

### Requirement: Private executable signing inputs

Release signing SHALL copy a bounded, verified native build input into a separate
private executable before invoking a signer. It MUST preserve the retained build
input and its other hard links, reject unsafe or existing output objects, and
revalidate the resulting private regular file before packaging.

#### Scenario: Cargo output has another hard link
- **WHEN** a signer changes a prepared executable copied from a hard-linked Cargo output
- **THEN** the Cargo output and its other links remain byte-for-byte unchanged.

#### Scenario: Signing destination is unsafe
- **WHEN** the destination already exists, is linked, or is outside the checked private directory
- **THEN** preparation fails without replacing it or modifying the build input.

### Requirement: Verified native publisher signatures

Every new macOS release executable SHALL carry a Developer ID Application
signature for the configured Apple team, hardened runtime and a secure timestamp,
and its notarization submission SHALL receive Accepted status. Every new Windows
release executable SHALL carry valid Authenticode for the configured publisher
and a timestamp from Azure Artifact Signing. Missing credentials or verification
failure MUST prevent final packaging and release promotion. Linux archives SHALL
retain their existing verified checksum contract without a mandatory OS signing
service.

#### Scenario: Ad hoc macOS signature
- **WHEN** a binary has a structurally valid ad hoc signature
- **THEN** production verification rejects it as lacking the configured Developer ID identity.

#### Scenario: Windows ARM64 executable
- **WHEN** an ARM64 Windows executable needs Azure signing
- **THEN** a supported Windows x64 runner signs it and native ARM64 acceptance verifies those exact bytes.

#### Scenario: Signing accounts are not configured
- **WHEN** a release is dispatched without required signing credentials or publisher configuration
- **THEN** preflight names the missing configuration and fails before stamping a version commit.

### Requirement: Accurate signing acceptance claims

Verification records and documentation SHALL distinguish fixture/native local
verification from production Apple notarization and Azure Public Trust signing.
They MUST NOT claim successful live signing without an observed service result.
Documentation SHALL explain that standalone macOS CLI binaries cannot have
notarization tickets stapled and that Authenticode does not guarantee SmartScreen
reputation.

#### Scenario: Account setup follows implementation
- **WHEN** the maintainer postpones production account setup
- **THEN** source implementation may complete while live service acceptance is explicitly deferred.
