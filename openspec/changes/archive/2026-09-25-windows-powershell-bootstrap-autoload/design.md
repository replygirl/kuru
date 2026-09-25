## Context

Stock Windows PowerShell 5.1 resolves an unloaded cmdlet through module
auto-discovery. Discovery walks the reconstructed module path in order (user,
Program Files, then `$PSHOME`) and, for modules missing from the per-profile
analysis cache under `LOCALAPPDATA`, analyzes them, including through a nested
`Get-Module -ListAvailable`. The Phase 0 CDB/SOS capture (PR #19 run
34801590851, job 103845193405) shows that path blocked in
`LoadModuleManifest -> StreamReader.ReadToEnd -> NtReadFile`. Which manifest
read pends, and why, was not captured then and is still unknown.

The installer's first non-Core command is `Write-Verbose` (Utility), followed
later by `Add-Type`, JSON conversion, `Write-Output`/`Write-Warning` (Utility)
and `Join-Path` (Management). Every failed native run stops at that first
command after a CPU burst, then waits. Passing runs on byte-identical script
lines show the stall is intermittent. The isolated fixtures clear the environment
and give each run a fresh `LOCALAPPDATA`, so every run starts with a cold cache.
That matches a user's first run of the saved script on a new profile.

## Goals / Non-Goals

**Goals:**

- The installer never reaches its own stock commands through module
  auto-discovery.
- Keep the `-Verbose` stage contract, native bridge and publication behavior.
- Provide a portable ordering/inventory contract and a discriminating native
  regression.

**Non-Goals:**

- Identify the manifest whose read pends or its kernel cause.
- Change fixture or product timeouts, pre-warm analysis caches, or give fixtures
  a shared/warm `LOCALAPPDATA`. That would hide the new-profile defect.
- Disable autoloading for anything outside the installer, or change the
  ToolHost, generic commands, MCPs, or CI-only PowerShell helpers.

## Decisions

- Use `Microsoft.PowerShell.Core\Import-Module` with `[IO.Path]::Combine($PSHOME, ...)`
  manifest paths, matching the proven ToolHost preamble. Rejected: module-qualified
  cmdlet names, which still resolve the module by name through the module path
  and could bind a shadowing copy. Also rejected: a fixture-only warm-up, which
  leaves real users exposed.
- Pass `-Verbose:$false` to the imports. The script is `[CmdletBinding()]`, and
  under `-Verbose` an import would otherwise emit one line per exported command
  into the documented stage output.
- Add flushed direct checkpoints after each import. They use the existing
  fixed-text marker form and never print paths.
- Use a portable source contract for ordering and inventory. It tokenizes
  `Verb-Noun` commands outside the C# bridge here-string, so adding a command
  from another module fails with a directive to import it. Use a native
  regression with `$PSModuleAutoLoadingPreference = 'None'` so that any
  remaining dependence on discovery fails deterministically instead of by timing.

## Risks / Trade-offs

- [Two explicit imports add finite startup work] -> They load the same modules
  first use would load, without enumerating other modules.
- [A stock image without these manifests] -> The import fails fast under
  `-ErrorAction Stop` with PowerShell's error. Autoload of the same modules would
  also have failed.
- [Residual unknown pending read in other discovery callers] -> Out of scope.
  If an installer stall recurs after the imports, the new checkpoints localize
  it, and the Phase 0 debugger preparation (3e811d5d) can be restored on a
  diagnostic branch.

## Integration contract

The installer resolves only manifests beneath the running stock PowerShell's
`$PSHOME`. It adds no route, SDK, schema or identifier contract. The `irm | iex`
route imports into the caller's session the same stock modules that its own
`irm` and first-use autoload would load. Generic configured commands and MCPs
keep their caller module policy.

## Operational surface

There is no bind address, container, connection limit, or required secret. The
change runs inside the user's or runner's x64 stock Windows PowerShell 5.1
(`powershell.exe` under `System32\WindowsPowerShell\v1.0`) and loads only the
Management and Utility manifests shipped beneath its `$PSHOME`. It is exercised
natively on the `windows-2025` runner's delivery-archive shard, application
shard, and installation job. No debugger or other diagnostic tooling enters the
product path.
