# Design

## Context

`scripts/install.ps1` is `[CmdletBinding()]` and is invoked with `&` in the
caller's session. Its first non-Core command is `Split-Path` (Management) at
the top of the body; its only other non-Core command is `Join-Path`
(Management) on the release-forwarding branch. Under Windows PowerShell 5.1, a
first use of a cmdlet from an unloaded module triggers
`CommandDiscovery.TryModuleAutoDiscovery`, which on a cold per-profile
module-analysis cache (under `LOCALAPPDATA`) analyzes every module on the
default path. `windows-powershell-bootstrap-autoload` measured this stalling
the release bootstrap and fixed only that script.

## Goals / Non-Goals

**Goals:**
- The entrypoint never reaches a non-Core stock command through discovery.
- A deterministic native regression that fails on the old script.
- A portable contract that catches regressions without Windows.

**Non-Goals:**
- Deadline, retry or sleep changes in any fixture.
- `PSModulePath` handling in the entrypoint.
- `apps/kuru-tui/support/install-source.ps1`, which has the same exposure
  (first non-Core `Join-Path`) behind the owned mise launch; that is separate
  work.
- Editing the existing source-entrypoint probe body (see Decisions).

## Decisions

- **Exact-path Core-qualified import, copied byte-for-byte from the release
  bootstrap.** Module-qualified cmdlet names were rejected there because they
  still resolve the module by name through `PSModulePath`. Identical text lets
  one portable contract match both scripts. `-Verbose:$false` keeps a caller's
  `-Verbose` free of import noise. The release bootstrap's direct stderr
  checkpoints are not copied: the entrypoint has no `-Verbose` stage contract.
- **Utility is imported although the entrypoint uses only Management.** The
  import lands in the global session state (the release bootstrap's native
  test on main already relies on this), so callers' later Utility commands,
  including the probe's `ConvertTo-Json` and `Write-Output`, resolve without
  discovery.
- **No `PSModulePath` change.** The script is not an owned launch; stripping
  would mutate the user's interactive shell. Exact `$PSHOME` paths do not read
  it, and `//apps/kuru-tui:install` already sets `env.PSModulePath = false`
  before launching stock PowerShell.
- **Probe left unchanged.** A pre-entrypoint import in the probe would mask
  the product defect; a `finally` guard or rewrite of `ConvertTo-Json` /
  `Write-Output` collides textually with #161. The residual path that could
  still discover in the probe is an entrypoint parameter-binding failure,
  which neither existing test exercises.
- **Native regression by autoload policy, not timing.** With
  `$PSModuleAutoLoadingPreference = 'None'`, the old script fails at
  `Split-Path` with command-not-found; the fixed script loads both exact
  manifests and reaches the fixture mise. Rejected: a timing assertion (flaky,
  needs a deadline) and asserting on #161's trace (unmerged).
- **New test appended with its own layout helper** instead of refactoring
  `source_entrypoint_fixture`, to stay outside #161's hunks.

## Risks / Trade-offs

- [`& mise` under autoload `None`] Application lookup precedes module
  auto-discovery, so it is unaffected (inference from PowerShell's command
  resolution order). If wrong, the native test fails on both old and new
  scripts at the mise call, which is visible rather than silent.
- [Intermittent stall] The historical stall is intermittent, so one green CI
  run does not prove the flake gone; #161's trace (once merged) names the
  statement of any recurrence.
- [PowerShell 7] `$PSHOME` there also ships both manifests; the documented
  route is Windows PowerShell 5.1, and pwsh is not exercised.

## Operational surface

There is no bind address, container, connection limit or required secret. The
entrypoint runs in the user's or runner's stock Windows PowerShell 5.1 session
(`powershell.exe` under `System32\WindowsPowerShell\v1.0`) and loads only the
Management and Utility manifests shipped beneath its `$PSHOME`. The native
regression runs in the `windows_cli` application tests on the native Windows CI
legs (windows-latest and windows-11-arm). No diagnostic tooling enters the
product path.

## Integration contract

The entrypoint binds to `[IO.Path]::Combine($PSHOME, 'Modules\<name>\<name>.psd1')`
for `Microsoft.PowerShell.Management` and `Microsoft.PowerShell.Utility`, the
manifests stock Windows PowerShell ships. Import failure stops the script
(`-ErrorAction Stop`) before any environment change or mise call. The native
regression asserts exactly one loaded module of each name whose `Path` equals
that manifest (ordinal, case-insensitive).
