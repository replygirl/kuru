## 1. Exact stock-module bootstrap

- [x] 1.1 Import the exact `$PSHOME` manifests for
  `Microsoft.PowerShell.Management` and `Microsoft.PowerShell.Utility` through
  `Microsoft.PowerShell.Core\Import-Module` and .NET path combination before
  user source, with `-ErrorAction Stop` and no change to output, exit, cleanup,
  autoload, or deadline behavior.
- [x] 1.2 Extend the existing fresh-state native ToolHost fixture to observe both
  exact modules loaded at user entry and complete unqualified `Join-Path` and
  `Get-FileHash`, retaining the selected CDB and known-sleep proof for the first
  native run.

## 2. Owning documentation

- [x] 2.1 Update `apps/kuru-docs/reference/tools.md`, `docs/protocols.md`, and
  `AGENTS.md` with the narrow built-in ToolHost stock-PowerShell bootstrap and
  unchanged general module-path/configured-command policy.

## 3. Evidence

- [x] 3.1 Pass focused connector tests, connector typecheck and clippy,
  formatter, documentation checks, strict Cospec validation, and the actual
  apply gate.
  The initial host checks passed, but a focused Rust reproduction exposed that
  `format!(concat!(..., "{command}"))` cannot use implicit capture from an
  expanded format string. After adding the explicit `command = command`
  argument, that reproduction compiled and ran. The two focused CDB tests,
  connector typecheck, connector clippy with warnings denied, workspace
  formatting, diff checks, documentation/content checks, strict validation,
  and actual apply gate passed.
- [ ] 3.2 On native Windows, pass the fresh-state ToolHost fixture with both
  exact modules loaded, unqualified stock cmdlets, known-sleep CDB proof,
  confirmed cleanup, and the ordinary connector shard receipt.
- [ ] 3.3 Pass the exact-head native matrix, four-receipt Windows aggregate and
  90% workspace line-coverage gate, Windows installation/offline checks, both
  Unix native jobs, both native builds, and all repository quality checks.
