## 1. Actionable connection diagnostics [critical]

- [x] 1.1 @regression (agent) run the existing real-Dolt wrong-directory/project identity rejection with cause assertions before and after the fix -> RED observed missing checked phase/cause after actual cleanup, /tmp/kuru-memory-diagnostics-red.log (5.77 seconds). GREEN retained safe filesystem/SQL identity causes, downcastable original PoolTimedOut and no fixture secrets, /tmp/kuru-memory-diagnostics-green.log (5.42 seconds). The existing missing-directory setup was preserved and its assertion accurately names filesystem validation failure.
- [ ] 1.2 @integration (agent) run real memory backup/restore and ownership tests -> identity verification, cleanup and durable history remain correct within existing deadlines.
- [ ] 1.3 @runtime (agent) inspect native Windows CI at the corrected source -> native memory checks pass or expose the actual failing phase; no unobserved root cause claimed.
