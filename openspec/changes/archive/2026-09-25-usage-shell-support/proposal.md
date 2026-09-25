## Why

Kuru currently generates shell completions with Clap's completion generator and renders its manual with separate Clap man-page code. Usage provides richer completion behavior while accepting Kuru's existing Clap command tree, so the command declaration can remain the single source of truth.

## What Changes

- Generate Bash, Zsh, Fish and PowerShell scripts and the manual from a Usage spec converted directly from the current Clap command tree.
- Make Usage's self-contained native scripts call a hidden, pure Kuru completion endpoint by default, backed by Usage's public completion-answer library API. Offer an explicit external Usage mode for users who prefer that executable.
- Preserve pure early generation, the exact five-file target-paired release sidecar and existing install, update and repair behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

- `shell-support`: Usage-backed shell generation, optional external helper, and Kuru-hosted completion answers.

## Impact

The TUI CLI, its owning tests and installation documentation change. The workspace pins compatible `usage-lib`, `usage-cli` and `usage-argv` Rust crates and updates Cargo.lock; obsolete completion and man-generation dependencies are removed if no other consumer remains. The five support filenames and release archive inventory do not change. This is not a CLI parser or command-syntax migration.

## Surfaces

- [x] interactive — shell completion and manual CLI output
- [ ] deploy — no release topology change
- [x] integration — Usage Rust library and optional external Usage executable protocol
- [ ] agent-behavior — no agent behavior change
