## Why

Release run 34411118843 passed its full validation but stopped in version
preparation because tracked files outside Cargo.toml and Cargo.lock changed.
Although mise-action installed Rust and hk with `--locked`, subsequent nested
`mise run` calls implicitly installed the remaining root tools without frozen
locks. A fresh local reproduction added a Taplo checksum to mise.lock.

## What Changes

- Carry frozen mise lockfile behavior through every nested release task using
  workflow environment, preserving the checked source tree.
- Disable implicit tool installation in bump, native build and publish jobs;
  their explicit locked Rust/hk setup supplies the tools they actually execute.
- Retain existing full setup for validation, notes and docs jobs, including Node
  activation before documentation tasks run.
- Detect unexpected setup changes before stamping and include offending paths in
  the native commit guard's diagnostic, without allowing additional files.

## Capabilities

### Modified Capabilities

None. The existing release source-integrity contract remains unchanged.

## Impact

.github/workflows/release.yml, native release diagnostics and their existing
integration fixture, docs/release.md, and this cospec record. No pins, credentials,
archive formats, publication order, repository visibility or application behavior
change. README direct installation and Dolt remain separate follow-ups.

## Surfaces

- [ ] interactive
- [x] deploy
- [x] integration
- [ ] agent-behavior
