## 1. Verified native publication [critical]

- [ ] 1.1 @regression (agent) repeat actual installed update after CI 34602482369 failed acknowledgment following successful publication at 11.7 seconds -> corrected native installed update succeeds and preserves offline memory; pre-fix evidence is job 103272899531 in /tmp/kuru-native-auth-ci-job-103272899531.log.
- [ ] 1.2 @integration (agent) hold the actual trusted helper's observed publication past the startup timeout, release it and inspect acknowledgment and files -> exact new bytes are acknowledged while the old parent remains alive, with original source and recovery identity retained.
- [ ] 1.3 @integration (agent) drive the production decoder with compiled native pipe fixtures to withhold acknowledgment and truncate or oversize its frame -> first-byte wait and frame completion fail within their independent bounds without accepting malformed publication.
- [ ] 1.4 @e2e (agent) run existing native update recovery, packaged offline install/update and unchanged coverage gates -> Windows checks pass with at least 90% coverage; Linux/macOS remain passing.
