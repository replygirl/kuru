# Proposal

## Why

The user requests a 95% minimum across every existing coverage gate. Checked
main `a549af91` reports 94.72% on Ubuntu, 94.73% on macOS and 93.69% on Windows,
so the stronger standard requires useful behavioral coverage as well as new
thresholds. Native reports expose gaps in private publication, cache and owner
recovery, provider receipts and failure handling that deserve direct tests.

## What Changes

- Enforce 95% in combined local coverage, each instrumented native OS merge,
  and the existing standalone archive and platform coverage tasks.
- Bind standalone coverage launches to mise's exact managed binary; a globally
  installed Cargo subcommand must not silently select another coverage version.
- Preserve the current cargo-llvm-cov line metric, complete source inventory,
  exact-test partitioning, native support checks and 95% boundary semantics.
- Expand tests around measured high-risk uncovered paths, using isolated native
  filesystem/process/IPC/database and HTTP/provider fixtures as appropriate.
- Reproduce and correct the legacy tool-receipt error flag loss, then check the
  request observed by a synthetic provider and malformed receipt refusals.
- Correct other reproduced defects within those tested contracts if found;
  distinguish proven bugs from coverage-only additions.
- Update the current repository policy and owning docs to the 95% standard.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `repository-delivery`: a consistent 95% meaningful line minimum for the
  workspace, every instrumented native OS, and existing package coverage gates.

## Impact

The affected sources and tests belong to `kuru-platform`, `kuru-memory`,
`kuru-runtime`, `kuru-connectors`, `kuru-delivery`, `kuru-archive` and `kuru-tui`.
Coverage policy lives in the delivery package's merge tool, package-owned mise
tasks, workflow expectations, AGENTS.md, and contributor/release documentation.
There are no tool or dependency upgrades, lockfile changes, storage migrations,
new runtime dependencies, HTML roadmap edits or publication actions.

## Surfaces

- [x] interactive — observable tool, terminal and command failure behavior
- [x] deploy — native CI coverage and package-owned verification tasks
- [x] integration — native ownership and provider/protocol fixture contracts
- [x] agent-behavior — accurate failed-tool receipt classification to inference
