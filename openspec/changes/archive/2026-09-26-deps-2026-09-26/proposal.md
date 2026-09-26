# Proposal

## Why

Several exact pins trail their latest stable releases as of 2026-09-26. Moving
them together keeps Cargo, mise and npm lockfiles, provenance records and the
dependency audit current in one reviewed change, superseding the separate
Dependabot updates.

## What Changes

- Cargo workspace pins: `clap` 4.6.7, `rustix` 1.1.5, `tiktoken-rs` 0.12.1 and
  `jsonschema` 0.58.0, with only their Cargo.lock entries moving.
- Root mise tools: mr-boxington 1.18.0 and cospec 0.8.2, with the regenerated
  cospec-managed files and the task-scoped cospec compatibility preload
  retargeted to the embedded OpenSpec 1.13.1 bundle.
- Delivery package: Communiqué 1.4.2.
- Docs app: Node 26.10.0, npm 12.1.0 (with its tarball checksum),
  `vitepress-plugin-llms` 1.14.0, `oxfmt` 0.70.0 and `oxlint` 1.85.0, with the
  npm lockfile refreshed.
- Five-platform mise lock refreshes at root, docs and delivery scopes, and the
  dependency audit, development, release and third-party notice text.
- Communiqué 1.4.0 replays signed thinking blocks natively, which trips the
  delivery canary that asserted the native adapter rejects them. The test is
  inverted to assert acceptance with the same fixture; the release configuration
  keeps its compatibility route. This is the one test change in this chore.
- mise stays at `min_version` 2026.9.4: 2026.9.14 is not yet installable through
  the maintainer's package manager and its CI pins are coupled to workflow and
  published-Windows verification code.

## Impact

Cargo.toml, Cargo.lock, mise.toml and mise.lock at root, docs and delivery
scopes, apps/kuru-docs package manifests, cospec-managed harness files and
schemas, packages/kuru-delivery/support/cospec-preload.cjs, the delivery
release-notes canary test, communique.toml
commentary, packages/kuru-connectors/THIRD_PARTY_NOTICES.md and docs
(dependencies, development, release). No runtime behavior or public contract
changes are intended.
