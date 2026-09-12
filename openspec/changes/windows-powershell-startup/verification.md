## 1. Native PowerShell startup [critical]

- [ ] 1.1 @regression (agent) reproduce the causal condition with real stock PowerShell 5.1 and compare the correction -> native failure before and pass after, with the cause recorded rather than inferred from a timeout
- [ ] 1.2 @integration (agent) exercise both direct and configured native launches in the isolated Unicode working directory -> actual .NET initialization, exact output, script marker and quiescent owned process tree within existing budgets
- [ ] 1.3 @integration (agent) run Windows platform coverage -> native regressions pass and package line coverage remains at least 90%

## 2. Repository checks

- [ ] 2.1 @integration (agent) run affected format, strict Windows-target Clippy and cospec validation -> checks pass without dependency or workflow changes

## Initial observation

At released commit `5afcdf463d5abaa586749ef2225b722373826823`, CI run `34630641990`, job `103366340487`, failed at `windows_commands.rs:227` with `Elapsed(())` while awaiting concurrent stdout/stderr reads. Job `103366340193` passed the identical test. Neither observation identifies the failing script stage or whether the root process was still running. Native reproduction and causal verification remain outstanding.
