## Why

Kuru's owned Windows PowerShell 5.1 shell reconstructs the stock module path,
but a first unqualified stock cmdlet can still enter module auto-discovery and
remain inside manifest analysis until the shell deadline. Native CDB evidence
captured the console host waiting for command execution while a managed worker
resolved `Join-Path` through `CommandDiscovery`, `AnalysisCache`, nested
`Get-Module -ListAvailable`, and `LoadModuleManifest`; the exact reason that
manifest read stayed pending is not established.

The shell must make its two required stock modules available before user source
begins, without depending on first-command discovery, changing user output, or
weakening its existing deadline and cleanup behavior.

## What Changes

- Import the exact stock `Microsoft.PowerShell.Management` and
  `Microsoft.PowerShell.Utility` manifests from `$PSHOME` through the already
  loaded `Microsoft.PowerShell.Core\Import-Module` command and .NET path
  combination before the user command.
- Fail the shell normally if either exact bootstrap import fails, while keeping
  output/error/exit projection, unrelated module autoloading, process cleanup,
  and every product deadline unchanged.
- Extend the isolated native fixture to use fresh local state, observe both
  exact modules loaded when user source begins, and execute unqualified
  `Join-Path` and `Get-FileHash`. Retain the bounded CDB capture for the first
  native proof rather than claiming the captured manifest-read cause is fully
  known.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing Windows shell contract already requires the owned stock
shell to reconstruct and use its compatible module environment; this corrects
the implementation of that contract.

## Impact

The production change is limited to the owned stock-shell preamble in
`packages/kuru-connectors/src/tools.rs`. Its native fixture remains in the same
file. `apps/kuru-docs/reference/tools.md`, `docs/protocols.md`, and `AGENTS.md`
document the narrow ToolHost boundary. Generic configured commands, MCP
commands, caller-supplied module paths, dependencies, public APIs, and release
behavior are unchanged.

## Surfaces

- [x] interactive — corrects user shell-tool startup on stock Windows
  PowerShell 5.1
- [ ] deploy — deploy/runtime/CI-execution topology
- [x] integration — binds the owned shell to exact modules shipped under the
  stock PowerShell `$PSHOME`
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
