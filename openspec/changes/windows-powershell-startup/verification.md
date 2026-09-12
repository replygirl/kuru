## 1. Bounded native capture and cleanup [critical]

- [ ] 1.1 @regression (agent) run a real native child that writes beyond a stream limit while holding the other stream open -> old capture reaches the overall timeout; corrected capture reports the output-limit error before that timeout, retains bounded bytes, and reaps the owned process
- [ ] 1.2 @integration (agent) run a real native child that emits known output and stalls -> capture reports its timeout with partial output and retained root/job state, then awaits owned process and pipe cleanup
- [ ] 1.3 @integration (agent) run the original stock PowerShell 5.1 script once through direct and configured native launches -> actual .NET initialization, exact output, script marker, and quiescent owned process tree within the unchanged 30-second capture budget

## 2. Repository and native acceptance

- [ ] 2.1 @integration (agent) run Windows platform coverage with the ordinary single CI job -> all native capture controls and PowerShell regressions pass; package line coverage remains at least 90%
- [ ] 2.2 @integration (agent) run affected formatting, strict Windows-target Clippy, type checks, and cospec validation -> checks pass with exact dependencies and normal CI topology preserved

## Historical failure and diagnostic evidence

At released commit `5afcdf463d5abaa586749ef2225b722373826823`, [CI run 34630641990, job 103366340487](https://github.com/replygirl/kuru/actions/runs/34630641990/job/103366340487) failed at `windows_commands.rs:227` with `Elapsed(())` while awaiting concurrent stdout/stderr reads. Job `103366340193` passed the identical test. Neither observation identifies the failing script stage or whether the root process was still running.

At `7cd0c330bc429f25282d7da8c205b78bb0988518`, job `103508201472` in [CI 34676884004](https://github.com/replygirl/kuru/actions/runs/34676884004) passed all 60 native platform tests and the coverage gate. The six command cases, including both diagnostic PowerShell launches, completed in 2.65 seconds. The unchanged released test also passed when the original failed job was rerun as `103509514713` in CI `34630641990`.

At `837ad3c`, [CI 34677891472](https://github.com/replygirl/kuru/actions/runs/34677891472) passed Windows platform job `103510958568`, including 64 repeated PowerShell launches. The instrumented workspace coverage step in Windows job `103510958780` passed another 64 launches. The earlier local pre-push hooks also passed, including workspace coverage. These observations precede the final capture correction and do not establish its acceptance.

At `2c22b280de6d4f836449fb90f4778af8c60763d6`, [CI 34702087044](https://github.com/replygirl/kuru/actions/runs/34702087044), platform job `103575526668`, passed the original unmodified PowerShell script repeated through 32 direct/configured pairs.

At `1d6a4c1ccfb66e56b1844d52c73a1edf16b0dc83`, [CI 34702657009](https://github.com/replygirl/kuru/actions/runs/34702657009) passed all 16 native platform jobs on fresh Windows runners. The parent agent observed the 16 successful jobs in GitHub's run metadata; the complete workflow was still running at that observation. Temporary repetitions and runner fan-out are removed from final CI.

The historical rare PowerShell stall remains unconfirmed. The concrete correction under acceptance is prompt output-limit failure and awaited cleanup; no passing diagnostic run is evidence that the original stall was fixed.

## Final correction evidence

The implementation and native failure-control checks are pending. Record observed before/after regression results, the final native run and head, cleanup assertions, coverage, and static checks here before marking the acceptance rows complete.
