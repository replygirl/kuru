# Proposal

## Why

Refresh every dependency declared in Cargo.toml and mise.toml to its latest available stable release, preserving exact pins and verified build inputs. The user explicitly excludes mise itself from this update.

## What Changes

- Refresh workspace Cargo.toml exact pins and Cargo.lock, and audit inherited app/package manifests.
- Refresh tool pins and mise.lock files owned by the root, docs app, memory package and delivery package; synchronize rust-toolchain.toml and existing Rust component selectors.
- Verify updated standalone cospec behavior, regenerate its managed integrations with the pinned CLI, and remove the compatibility preload only after its contract passes without it.
- Refresh the source-built Windows ARM64 Dolt recipe's Go toolchain and verified output inventory, and update dependency/provenance documentation and the canonical AGENTS.md workflow guide.

## Impact

Builds, installation helpers, native test fixtures, docs tooling and CI use the refreshed exact dependencies. Preserve mise version requirements, CI mise pins, package task ownership, private asset verification, normal hooks and the 90% coverage gate; no release publication is requested.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
