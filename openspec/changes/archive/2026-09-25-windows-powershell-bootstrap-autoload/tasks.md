## 1. Correct the installer bootstrap

- [x] 1.1 Add a portable regression contract that fails on the unfixed installer (no exact PSHOME imports before `Write-Verbose`) and passes after, and verify it rejects both an early stock command and a command from an unimported module
- [x] 1.2 Import exact PSHOME Management and Utility manifests through `Microsoft.PowerShell.Core\Import-Module` before the first non-Core command, with flushed fixed-text checkpoints, and verify the portable contract passes
- [x] 1.3 Add the native autoload-disabled installer regression, remove the fixture wrapper's bare `Join-Path`, extend the application fixture's ordered checkpoint list, and verify the Windows-only test sources type-check for the MSVC target where the host allows

## 2. Document and deliver

- [x] 2.1 Update `docs/install.md`, `apps/kuru-docs/guide/installation.md` and `AGENTS.md` for the installer's module bootstrap and verify docs checks pass
- [x] 2.2 Run focused delivery checks, strict Cospec validation/apply and normal commit hooks; record observed results and name unrun native Windows checks
