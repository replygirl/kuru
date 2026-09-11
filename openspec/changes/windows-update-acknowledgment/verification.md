## 1. Verified native publication [critical]

- [x] 1.1 @regression (agent) repeat actual installed update after CI 34602482369 failed acknowledgment following successful publication at 11.7 seconds -> CI 34607501091, job 103289452218, passed the actual Windows embedded install/update fixture in 134.46 seconds, preserving separate cold offline memory caches. All 12 native updater tests passed. Log: /tmp/kuru-windows-resume-ci-job-103289452218.log. Pre-fix evidence is job 103272899531 in /tmp/kuru-native-auth-ci-job-103272899531.log.
- [ ] 1.2 @integration (agent) hold the actual trusted helper's observed publication past the startup timeout, release it and inspect acknowledgment and files -> exact new bytes are acknowledged while the old parent remains alive, with original source and recovery identity retained.
- [ ] 1.3 @integration (agent) drive the production decoder with compiled native pipe fixtures to withhold acknowledgment and truncate or oversize its frame -> first-byte wait and frame completion fail within their independent bounds without accepting malformed publication.
- [ ] 1.4 @e2e (agent) run existing native update recovery, packaged offline install/update and unchanged coverage gates -> Windows checks pass with at least 90% coverage; Linux/macOS remain passing.

CI 34607501091 passed the slow-publication and bounded native pipe fixtures at
8cb4738. A later review corrected test cleanup and replaced Job-based liveness
observation with a retained root process handle in e080f84; that revised fixture
still requires native execution. The same CI run passed every static category
and all four Unix targets, but the Windows shell and browser controls failed,
so final Windows coverage, source installation and shipping inspection were
not reached. These are pending gates, not inferred passes.
