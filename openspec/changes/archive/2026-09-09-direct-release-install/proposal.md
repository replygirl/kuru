## Why

The README leads with compiling a checkout, and the current release-install
shell entrypoint itself requires Cargo. Users should be able to install the
published native executable through mise or one shell command without a Rust
toolchain. Installation documentation should describe the commands directly,
without repository-visibility commentary or future-readiness framing.

## What Changes

Add a small Bash 3.2-compatible bootstrap owned by kuru-delivery. It resolves a
release, downloads and checks the matching platform archive, extracts only the
regular executable into private staging, and atomically installs it. Preserve
the source installer and native Rust updater. Document mise GitHub installation
and the compiler-free shell command before source installation in the README,
and align both installation guides and related operational links.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `repository-delivery`: direct binary installation, release selection and installation documentation.
- `release-automation`: installation guidance for released native binaries, without visibility-dependent instructions.

## Impact

Package-owned shell bootstrap and Rust-driven integration tests, delivery mise
tasks, the existing shell entrypoint, README, installation guides, linked
release/contributing/security documentation and docs navigation label. No new language runtime, dependency, root workspace,
release asset format, publication workflow or storage migration is introduced.
Merge follows successful initial release publication and deployment. Independent
implementation may proceed during the external attestation-service outage without
altering the immutable source of the running release.

## Surfaces

- [x] interactive — installation CLI, errors and user instructions
- [x] deploy — executable download, validation and atomic installation
- [x] integration — GitHub release assets and mise GitHub backend
- [ ] agent-behavior — no model, prompt or routing change
