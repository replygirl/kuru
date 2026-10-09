# Design

## Context

The five native release targets already produce verified core archives and paired
shell support. Staged installation, previous-release updating and docs deployment
gate publication. Cargo outputs can be hard-linked, signing timestamps change
bytes, and Azure's action cannot run on Windows ARM64. The maintainer will provide
Apple/Azure accounts later and selected `replygirl/homebrew-kuru` for the tap.

## Goals / Non-Goals

Complete configurable signing and Homebrew delivery within the existing workflow,
preserve exact accepted bytes on retries, and prove local contracts with native
and HTTP fixtures. Do not change targets, dependencies, tool pins, archive member
formats, runtime engines, provider behavior, the HTML roadmap, or dispatch a
release. Do not introduce a general signer/plugin framework, an Apple installer
package, Homebrew bottles or source compilation.

## Decisions

- Keep signing preparation and verification in tooling-gated `kuru-delivery`.
  Existing checked build-input and private staging boundaries suffice; never sign
  Cargo's artifact in place or weaken cache/output privacy.
- macOS imports an ephemeral Developer ID keychain, signs with hardened runtime
  and timestamp, verifies the configured team, submits a temporary ZIP through
  `notarytool`, checks Accepted, and removes temporary key material. Raw CLI
  notarization cannot be stapled; no unsupported offline Gatekeeper promise.
- Build each Windows executable natively. A Windows x64 signing job signs both
  PE architectures through the official SHA-pinned Azure action and OIDC, verifies
  publisher/timestamp, and packages the result. ARM64 staged acceptance executes
  the exact resulting package natively.
- Retain final per-target artifacts bound to source/version/target in the same
  workflow run and restore them before signing on retry. Artifact absence permits
  first creation; conflicting or expired retained state fails visibly. Never
  overwrite a retained signed package or public release asset.
- Generate the binary formula from read-only verified candidate checksums.
  Pair support resources with every selected target, install completions/manual,
  and require installed bytes to match: Homebrew linkage fixups can otherwise
  replace signatures even when stripping is disabled.
- Gate release promotion on native Homebrew acceptance for its three supported
  targets. Isolate fixture taps/private application state; refuse changing an
  unrelated pre-existing Kuru installation.
- Update the dedicated tap only after publication, with a separate app and GitHub
  Contents API compare-and-swap. Read the actual public release's exact manifest,
  preserve identical content, and reject rollback/conflicting same-version bytes.
  Keep tap templates source-owned and bootstrap an initialized tap without adding
  a root Ruby project or another publication workflow.
- Ordinary native installation CI also accepts its actual source-installed
  executable through Homebrew before merge, without paid signing accounts. Its
  other-target archive shapes are explicit fixtures for formula selection;
  they establish no foreign architecture or publisher-signing claim. Release
  staged acceptance always consumes the actual complete release candidate.

## Risks / Trade-offs

Production Apple and Azure acceptance is explicitly deferred until account setup;
fixture signatures do not establish Public Trust or notarization success. Missing
credentials must fail release preflight before version mutation. Azure certificate
subjects must match configured publisher identity and short-lived certificates
require timestamps. Workflow artifacts have finite retention; expired retained
signed state needs an explicit recovery diagnosis rather than silent re-signing.
Homebrew availability and linkage behavior are established on native CI, not from
formula syntax alone. A tap update can fail after public promotion; retry repairs
the tap without changing the already public release.

## Operational surface

Everything runs in the existing GitHub Release workflow, on native macOS ARM64,
Linux x64/ARM64 and Windows x64/ARM64 runners. Signing Windows ARM64 is performed
on the x64 runner; no container is introduced. Runtime binds do not change.
Native acceptance uses only bounded loopback fixtures and private application
state. Apple secrets are the P12/password and notary P8; variables bind signing
identity, team, key and issuer. Azure uses job-scoped OIDC plus tenant/client/
subscription and signing endpoint/account/profile variables, not a static secret.
The tap uses a separately repository-scoped App key and ID. Missing configuration
fails preflight before stamping. Existing compiler/tool pins and target catalog
are retained; only newly required official workflow actions receive current SHA
pins. No user provider credential stores are accessed.

## Integration contract

Apple's native `codesign` and `notarytool` enforce Developer ID team, hardened
runtime, timestamp and Accepted JSON status. Azure's official action receives
private PE copies and RFC3161 timestamp configuration; stock PowerShell verifies
Authenticode against the configured publisher. GitHub artifact identity binds the
same run and selected source/version/target. The Homebrew formula uses exact
immutable GitHub release URLs with SHA256 digests; native acceptance substitutes
bounded local fixture URLs only. Tap GitHub Contents writes use Base64 UTF-8
formula bytes and the inspected blob SHA for compare-and-swap, through the
existing bounded authenticated API client. HTTP fixtures test real endpoint and
schema/error reconciliation, including immutable public release identity.
