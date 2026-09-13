## 1. Failure-only Windows diagnostic

- [x] 1.1 Add at most three fresh isolated controlled launches after the existing Kuru shell failure and verify they preserve the original success assertions.
- [x] 1.2 Run the relevant native fixture when scheduled and record only observed encoded/environment/console outcomes. Hosted PR18 `ba4a3b7` Windows full-application job `103756331991` passed the authoritative fixture. The failure-only controls did not run because the original acceptance did not fail; this records no diagnostic conclusion about the former flake.
