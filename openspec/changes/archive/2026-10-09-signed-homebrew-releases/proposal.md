# Proposal

## Why

Kuru's native release archives still lack macOS Developer ID signing/notarization
and Windows Authenticode, and Homebrew distribution remains deferred. The
maintainer has authorized completing both implementations while Apple and Azure
account provisioning happens separately, so release automation must be ready and
fail clearly when its required signing credentials are unavailable.

## What Changes

- Sign verified private executable copies before final archive/checksum creation:
  Developer ID and notarization on macOS, timestamped Azure Artifact Signing on
  Windows, with native verification and acceptance of the exact resulting bytes.
- Preserve signed build artifacts across release retries instead of re-signing
  previously accepted artifacts or replacing published assets.
- Generate and accept a Homebrew binary formula from the exact native archives
  and paired shell-support assets, preserving executable bytes and embedded Dolt.
- Update a dedicated public tap after publication using a separately scoped
  GitHub App, with idempotent, concurrency-checked updates and no version rollback.
- Document account setup, credential names, release recovery, installation and
  the distinction between fixture verification and later live signing acceptance.

## Capabilities

### New Capabilities

- `native-code-signing`: native executable signing preparation, verification and
  notarization within release automation.
- `homebrew-distribution`: verified binary formula generation, native installation
  and tap update automation.

### Modified Capabilities

- `release-automation`: signed artifact ordering, retry reuse and Homebrew updates
  through the existing single manual release workflow.

## Impact

Affected surfaces are `packages/kuru-delivery` helpers/tests/support tasks,
`.github/workflows/release.yml`, ordinary native installation CI, repository workflow contract tests, and owning
installation/release/verification documentation. No provider, actor, memory,
archive format, runtime dependency or existing tool version change is intended.
Production Apple/Azure credentials and tap-app configuration are external setup
prerequisites, deliberately unavailable during this implementation; fixture and
native local acceptance must not be represented as live service acceptance.
The existing five release targets remain unchanged. Linux needs no new native
code-signing service. No release is dispatched as part of implementation.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
