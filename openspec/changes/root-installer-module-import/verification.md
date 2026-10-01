# Verification

## 1. Entrypoint never discovers its own stock commands [critical]

- [ ] 1.1 @regression (agent) run `powershell_diagnostics::source_entrypoint_imports_pshome_modules_before_any_discovered_command` against the unfixed and fixed `scripts/install.ps1` -> fails on the unfixed script and on an injected early `Split-Path`; passes on the fixed script
- [ ] 1.2 @regression (agent) run native `windows_cli::source_install_entrypoint_supplies_its_stock_commands_without_module_auto_discovery` on stock Windows PowerShell 5.1 with a fresh isolated `LOCALAPPDATA` and autoloading disabled, in native Windows CI only (no local Windows host) -> fixed entrypoint reaches mise once with exactly one exact-PSHOME Management and Utility module loaded; the unfixed script fails at `Split-Path` with command-not-found

## 2. Install-timeout family no longer recurs [critical]

- [ ] 2.1 @runtime (agent) observe repeated native windows-latest runs of the `windows_cli` source-entrypoint tests at the fixed head, in native Windows CI -> no install timeout; one green run does not prove an intermittent stall fixed

- [ ] 2.2 @e2e (agent) run the existing `windows_cli` source-entrypoint tests (`source_install_entrypoint_scopes_first_mise_and_restores_environment_on_success_or_failure`, `source_install_entrypoint_rejects_release_options_before_mise_or_environment_changes`) through the real entrypoint in native Windows CI -> both pass with the probe unchanged, its `ConvertTo-Json`/`Write-Output` resolving from the entrypoint's global imports

## 3. Repository gates

- [ ] 3.1 @integration (agent) run format check, lint, Windows-target lint, typecheck, delivery tests and shell lint, docs check and strict Cospec validation/apply -> all pass
