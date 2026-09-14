## Context

Native stack text captured an owned stock PowerShell 5.1 shell after it emitted
the marker immediately before its first `Join-Path`. The console-host thread was
waiting for command execution while another managed thread ran
`TryModuleAutoDiscovery`, `AnalysisCache`, nested `Get-Module -ListAvailable`,
and `LoadModuleManifest` down to a native file read. This localizes the delayed
boundary to stock-command discovery, but it does not identify the manifest path
or prove why the read remained pending.

## Goals / Non-Goals

**Goals:**

- Make the stock Management and Utility commands available before user source.
- Preserve the owned ToolHost process, environment, output, exit, cleanup, and
  deadline contracts.
- Prove the correction with the existing real native fixture and first-run CDB
  observation.

**Non-Goals:**

- Disable normal PowerShell module autoloading or alter caller-configured
  commands and MCPs.
- Diagnose arbitrary manifests, inspect user stores, or change debugger scope.
- Claim the captured manifest read's underlying kernel or host cause is known.

## Decisions

- Build each manifest path from `$PSHOME` with .NET `Path.Combine` and import it
  through the already-loaded `Microsoft.PowerShell.Core\Import-Module` command.
  This avoids invoking a discoverable stock cmdlet before bootstrap.
- Import only `Microsoft.PowerShell.Management` and
  `Microsoft.PowerShell.Utility`, with `-ErrorAction Stop`, before appending the
  unchanged user source. Other commands keep ordinary autoload behavior.
- Extend the existing isolated native fixture with fresh local state, exact
  loaded-module observations, and unqualified `Join-Path` and `Get-FileHash`.
  Keep the selected CDB/known-sleep proof for the first hosted run.

## Risks / Trade-offs

- Exact imports add finite startup work to each built-in stock-shell call. Using
  two OS-shipped manifests avoids broad module enumeration and makes failure
  explicit.
- Preloaded module exports may affect command resolution. These are the stock
  modules PowerShell would otherwise autoload for the same commands, and user
  source may still define later commands normally.
- A passing fixture proves the discovery stall is avoided but does not establish
  the original manifest read's deeper cause. Evidence retains that limitation.

## Operational surface

There is no bind address, container, connection limit, or required secret. The
boundary runs inside Kuru's owned x64 stock Windows PowerShell 5.1 child on the
native Windows runner or user host. The temporary CDB input remains selected
only for the isolated connector fixture and does not enter production startup.

## Integration contract

ToolHost owns the launch and passes encoded source to the OS-shipped stock
PowerShell executable. The bootstrap resolves only manifests beneath that
process's `$PSHOME`; it adds no mount, route, SDK, schema, identifier, or data
reconciliation contract. The native fixture uses isolated fake inputs and fresh
APPDATA/LOCALAPPDATA roots, and generic configured commands retain their caller
environment and module policy.
