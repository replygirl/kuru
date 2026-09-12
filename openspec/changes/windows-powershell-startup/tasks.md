## 1. Establish the cause

- [ ] 1.1 Capture the direct/configured launch label, script progress, partial output and retained process state on failure, with awaited owned cleanup, and reproduce the failure on native Windows.
- [ ] 1.2 Identify the causal condition from native evidence and record it in the proposal; distinguish observations from hypotheses.

## 2. Correct and verify

- [ ] 2.1 Correct the established cause in the owning platform boundary or fixture without relaxing its execution contract, deadlines or coverage threshold.
- [ ] 2.2 Verify a meaningful regression fails with the causal condition and passes with the correction on native Windows, including direct and configured PowerShell 5.1 launches and actual .NET initialization.
- [ ] 2.3 Run the affected platform format/lint/type checks and native coverage, preserve the 90% gate, and record exact native CI evidence before archive.
- [ ] 2.4 Remove the temporary native CI reproduction matrix and stress repetitions before merge, preserving the ordinary platform job and coverage artifact.
