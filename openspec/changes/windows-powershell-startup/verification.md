## 1. Native PowerShell startup [critical]

- [ ] 1.1 @regression (agent) reproduce the causal condition with real stock PowerShell 5.1 and compare the correction -> native failure before and pass after, with the cause recorded rather than inferred from a timeout
- [ ] 1.2 @integration (agent) exercise both direct and configured native launches in the isolated Unicode working directory -> actual .NET initialization, exact output, script marker and quiescent owned process tree within existing budgets
- [ ] 1.3 @integration (agent) run Windows platform coverage -> native regressions pass and package line coverage remains at least 90%

## 2. Repository checks

- [ ] 2.1 @integration (agent) run affected format, strict Windows-target Clippy and cospec validation -> checks pass without dependency or workflow changes

## Initial observation

At released commit `5afcdf463d5abaa586749ef2225b722373826823`, CI run `34630641990`, job `103366340487`, failed at `windows_commands.rs:227` with `Elapsed(())` while awaiting concurrent stdout/stderr reads. Job `103366340193` passed the identical test. Neither observation identifies the failing script stage or whether the root process was still running. Native reproduction and causal verification remain outstanding.

## Diagnostic observations

At `7cd0c330bc429f25282d7da8c205b78bb0988518`, job `103508201472` in CI `34676884004` passed all 60 native platform tests and the coverage gate. The six command cases, including both diagnostic PowerShell launches, completed in 2.65 seconds. The unchanged released test also passed when the original failed job was rerun as `103509514713` in CI `34630641990`. These passes do not establish a cause or a correction.

The focused reproduction now compares 32 fresh isolated working directories, each with a direct and configured process. Each launch retains its original 30-second capture budget, and the first failure retains its output, progress, root/job state and awaited cleanup result. This temporary repetition seeks a native failure with actionable evidence; it is not evidence of a fix.

At `837ad3c`, CI `34677891472` passed the Windows platform job `103510958568`, including all 64 repeated PowerShell launches. The instrumented workspace coverage step in Windows job `103510958780` also passed, exercising another 64 launches. All required local pre-push hooks passed from the separate worktree, including workspace coverage. No captured native timeout identifies the original cause yet; the cause and correction tasks remain open, and this change must not be archived or presented as a completed fix.
