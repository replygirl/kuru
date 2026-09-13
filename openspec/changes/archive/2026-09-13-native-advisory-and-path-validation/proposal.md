## Why

The advisory scanner deliberately accepts only the local configuration values
produced by a fresh owned clone. Git for Windows records the stock
`core.symlinks=false` value in that clone, but the narrow validator currently
rejects it before the scanner can run. The existing delivery and core tests
also compare Windows path spellings as Unix text, which rejects equivalent
canonical paths and the established escaped provenance display.

## What Changes

- Accept only the stock Git-for-Windows fresh-clone value
  `core.symlinks=false`, while retaining rejection of a true or unknown local
  configuration value.
- Compare advisory fixture working directories by canonical path identity and
  assert configuration provenance through its existing escaped-display
  contract.
- Add focused regressions for the accepted Windows bookkeeping value and its
  rejected unsafe alternative.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

<!-- None. -->

## Impact

- `packages/kuru-delivery/src/advisory.rs` and its unit/integration fixtures:
  exact fresh-clone Git configuration validation and native path assertions.
- `packages/kuru-core/tests/config.rs`: portable assertion of the existing
  escaped configuration-provenance display.
- No scanner command, advisory policy, production path representation, public
  configuration schema, or network behavior changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (Git fresh-clone local configuration and native filesystem path representation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
