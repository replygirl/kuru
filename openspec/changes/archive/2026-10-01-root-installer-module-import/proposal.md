# Proposal

## Why

The checkout source entrypoint `scripts/install.ps1` calls a bare
`Split-Path` (`Microsoft.PowerShell.Management`) as its first command outside
`Microsoft.PowerShell.Core`, with no module import before it. On a profile
whose Windows PowerShell 5.1 module-analysis cache is cold, that first use
enters command auto-discovery, which analyzes every module on the default path
and can stall indefinitely; `windows-powershell-bootstrap-autoload` measured
this signature (about 20 s of CPU, then a blocked file read) and fixed it in the
release bootstrap only, leaving other discovery callers out of scope. The
native `windows_cli` source-entrypoint probe runs this script under a fresh
`LOCALAPPDATA`, and its intermittent install-timeout family matches that
signature (an inference: no recorded stall has yet named the parked statement).
A user running `& .\scripts\install.ps1 -Source` on a new profile is exposed to
the same delay or stall.

## What Changes

- Before `Split-Path`, the entrypoint imports the exact
  `$PSHOME\Modules\Microsoft.PowerShell.Management` and
  `Microsoft.PowerShell.Utility` manifests through the always-loaded
  `Microsoft.PowerShell.Core\Import-Module`, with `-Verbose:$false` and
  `-ErrorAction Stop`. The two lines are byte-identical to the release
  bootstrap's, which become no-ops on the forwarding branch.
- The entrypoint keeps running in the caller's session and does not read or
  strip `PSModulePath`: it is not an owned launch, exact `$PSHOME` paths do
  not consult it, and the downstream owned launch (`//apps/kuru-tui:install`)
  already removes it.
- The portable source contract that guards the release bootstrap is
  generalized and applied to the entrypoint, so a non-Core command before the
  imports, or one from an unimported module, fails locally.
- A native Windows regression runs the real entrypoint with module autoloading
  disabled under a fresh `LOCALAPPDATA` and requires both exact manifests
  loaded and mise reached; on the old script it fails deterministically at
  `Split-Path` with command-not-found, without any timing assertion.
- The existing source-entrypoint probe body is not edited. Because the
  entrypoint's imports land in the global session state, the probe's later
  `ConvertTo-Json` and `Write-Output` resolve from loaded modules on every path
  its tests exercise; editing them would collide with the open
  `windows-probe-trace` change (#161), which rewrites that hunk.
- Installation documentation names the entrypoint's module bootstrap.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`scripts/install.ps1`; `packages/kuru-delivery/tests/powershell_diagnostics.rs`;
`apps/kuru-tui/tests/windows_cli.rs` (one appended native test and layout
helper); `docs/install.md` and `apps/kuru-docs/guide/installation.md`. No
dependency, product timeout, fixture deadline, retry, sleep, release asset,
`PSModulePath` or Rust runtime behavior changes. Fixtures keep a fresh
`LOCALAPPDATA`, which models a new profile. No durable spec describes the
installer's module handling (the release bootstrap fix carried no spec delta),
so no capability changes.

## Surfaces

- [x] interactive — first-run behavior of the documented `& .\scripts\install.ps1` route
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — binds the entrypoint to the exact modules shipped under stock PowerShell's `$PSHOME`
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
