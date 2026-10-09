# Verification

## 1. Native CLI termination [critical]

- [x] 1.1 @regression (agent) same existing Windows doctor error commands under runner instrumentation -> before76a076ea cases pass but their error branches have zero counters and doctor235/461; after normal-return correction those exercised branches are covered without new doctor tests or profile-flush APIs.
- [x] 1.2 @e2e (agent) real typed CLI commands including held stdin/output and interruption -> same status, diagnostics/privacy and bounded process exit after checked authority cleanup.

## 2. Boundary preservation

- [x] 2.1 @integration (agent) host/Windows static checks and independent review -> standard ExitCode only; existing worker join, routes, limits and ownership retained.

Observed: independent source reviews clear; application and all affected package Windows-target lint pass. Host CLI/doctor/interruption checks pass as detailed below; native counter comparison remains pending.
Observed host acceptance: doctor7/7, canary4/4 and held-stdin/held-output interruption2/2 pass. Existing statuses, private diagnostics and completed retry assertions remain. Native Windows doctor counter comparison1.1 remains unrun until CI.

Subsequent native acceptance: exact861811ef CI37872687339 partition3/8 succeeds with a valid export and the unchanged redacted invocation-error test. Its invocation_failure257–270 is now14/14 (saved76a076ea0/14), and DoctorExit::fmt234–236 is3/3 (saved0/3). Independent analysis confirms the same doctor source/test files and runner-owned profile destination; no test duplication or explicit flush API was added. Critical1.1 is satisfied.
