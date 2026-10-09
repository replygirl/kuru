# Verification

## 1. Native CLI termination [critical]

- [ ] 1.1 @regression (agent) same existing Windows doctor error commands under runner instrumentation -> before76a076ea cases pass but their error branches have zero counters and doctor235/461; after normal-return correction those exercised branches are covered without new doctor tests or profile-flush APIs.
- [x] 1.2 @e2e (agent) real typed CLI commands including held stdin/output and interruption -> same status, diagnostics/privacy and bounded process exit after checked authority cleanup.

## 2. Boundary preservation

- [x] 2.1 @integration (agent) host/Windows static checks and independent review -> standard ExitCode only; existing worker join, routes, limits and ownership retained.

Observed: independent source reviews clear; application and all affected package Windows-target lint pass. Host CLI/doctor/interruption checks pass as detailed below; native counter comparison remains pending.
Observed host acceptance: doctor7/7, canary4/4 and held-stdin/held-output interruption2/2 pass. Existing statuses, private diagnostics and completed retry assertions remain. Native Windows doctor counter comparison1.1 remains unrun until CI.
