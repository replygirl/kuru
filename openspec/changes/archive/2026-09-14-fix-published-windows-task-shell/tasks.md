## 1. Published Windows task boundary

- [x] 1.1 Replace the cmd-incompatible `run_windows` body with an explicit package-owned `pwsh.exe` entrypoint while preserving every verifier input, mise identity, exit status, and the non-Windows failure
- [x] 1.2 Add a native Windows regression that invokes the actual mise task with isolated missing verifier input and proves the PowerShell script's fail-closed validation is reached before Cargo or release behavior
- [x] 1.3 Pass focused delivery workflow/task tests, formatting, task configuration validation, strict Cospec validation, and diff checks; keep native GitHub execution as the required merge gate. *(Observed locally: four portable PowerShell diagnostics tests and the adjusted release-workflow test passed; delivery all-target/all-feature typecheck, Cargo formatting, Taplo validation, and diff checks passed. The actual Windows task regression remains pending required hosted CI.)*
