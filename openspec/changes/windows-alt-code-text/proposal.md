# Proposal

## Why

Actual Windows ConPTY input loses U+0301 before composer editing or recall: the native receipt contains the character only as a release, then Kuru correctly ignores keyboard releases. Crossterm's Windows parser recognizes an Alt-code text commit internally but labels it as an ordinary release, discarding the distinction needed for safe application handling.

## What Changes

- Patch only the existing Alt-code discriminator in the exact crossterm 0.29.0 source so committed text becomes Press; retain ordinary keyboard Release events and unchanged Kuru release filtering.
- Keep a verified upstream source copy and MIT license under `apps/kuru-tui/vendor/crossterm-0.29.0`, with original archive checksum and VCS provenance. Root Cargo applies one patch for all consumers and retains the exact version; exclude the foreign dependency from Kuru workspace membership, preserving the existing application source inventory.
- Strengthen the actual ConPTY input probe to check the accent is committed once and ordinary release events remain distinct, retaining exact seven-turn persisted Unicode bytes at both terminal sizes.
- Preserve the existing foreign-dependency coverage boundary with a filter restricted to this exact vendored dependency directory, shared by canonical exports and local workspace reporting; retain all application sources and the existing metric/gates.
- Document the dependency patch in the owning development page and keep repository vendor conventions accurate.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

TUI-owned vendored dependency source, root Cargo patch and lock, delivery coverage reporting/task, native fixture assertions, contributor documentation and AGENTS.md. Preserve the exact original foreign manifest through an exact-file Taplo exclusion; every Kuru manifest remains checked. No dependency/tool version upgrades, mise version changes, new input backend, keyboard-state heuristic, normalization, unsafe consumer exemption, threshold change, or HTML roadmap edit.

## Surfaces

- [x] interactive — actual Windows text entry
- [ ] deploy
- [x] integration — existing crossterm native parser contract
- [ ] agent-behavior
