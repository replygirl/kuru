## 1. Verified native publication [critical]

- [x] 1.1 @regression (agent) repeat actual installed update after CI 34602482369 failed acknowledgment following successful publication at 11.7 seconds -> CI 34607501091, job 103289452218, passed the actual Windows embedded install/update fixture in 134.46 seconds, preserving separate cold offline memory caches. All 12 native updater tests passed. Log: /tmp/kuru-windows-resume-ci-job-103289452218.log. Pre-fix evidence is job 103272899531 in /tmp/kuru-native-auth-ci-job-103272899531.log.
- [x] 1.2 @integration (agent) hold the actual trusted helper's observed publication past the startup timeout, release it and inspect acknowledgment and files -> CI 34612890798, Windows job 103307525914, passed publication_after_startup_budget_acknowledges_exact_image_while_parent_lives with the retained root handle after the eleven-second hold. Exact source/new/recovery bytes and identities remain asserted; log line 1114.
- [x] 1.3 @integration (agent) drive the production decoder with compiled native pipe fixtures to withhold acknowledgment and truncate or oversize its frame -> the same native job passed publication_wait_and_partial_frames_have_separate_bounded_native_deadlines at line 1112. It exercised actual private pipes and independent first-byte, partial-frame and total bounds, including malformed/truncated frames, with the revised complete cleanup path.
- [x] 1.4 @e2e (agent) run existing native update recovery, packaged offline install/update and unchanged coverage gates -> all 12 Windows updater cases passed in 114.81 seconds. Native source-installed Kuru passed direct install/self-update and conversations from distinct empty offline Dolt caches in 79.23 seconds (lines 1693–1696); both shipping images passed OS-only DLL inspection (1744–1745). Raw Windows workspace coverage was 19061/20227 (94.235428%) and platform coverage 2904/3178 (91.378225%). All four Unix targets and static checks passed; full CI completed successfully at ca8e38a.

CI 34607501091 passed the slow-publication and bounded native pipe fixtures at
8cb4738. A later review corrected test cleanup and replaced Job-based liveness
observation with a retained root process handle in e080f84. The same CI run passed every static category
and all four Unix targets, but the Windows shell and browser controls failed,
so final Windows coverage, source installation and shipping inspection were
not reached. CI 34612890798 subsequently exercised the revised fixture and every
native shipping gate successfully. Completed Windows log:
/tmp/kuru-shell-lookup-ci-job-103307525914.log; the complete run and raw coverage
record are in /tmp/kuru-shell-lookup-ci-report.md. Its shell result used diagnostic
observers; the separate plain-source shell follow-up is not counted as an updater
change or an already observed shell pass.
