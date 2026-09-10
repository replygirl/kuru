## 1. Complete offline installation [critical]

- [ ] 1.1 @e2e (agent) Launch only the packaged Kuru executable with empty offline cache and isolated data, run demo chat, reopen and inspect memory plus license hashes -> persisted real Dolt conversation without separate installation or download
- [ ] 1.2 @integration (agent) Exercise source/direct installation and native update, then launch the resulting executable with another empty offline cache -> every install path carries the complete matching runtime
- [ ] 1.3 @integration (agent) Run packaged runtime smoke on existing macOS/Linux architecture matrix -> actual conversation/reopen passes on every currently supported target; Windows is verified in the dependent native-windows change

## 2. Reproducible build preparation [critical]

- [ ] 2.1 @integration (agent) Prepare actual pinned local archive offline and build through mise from missing generated inputs -> verified exact target bytes embedded with licenses and no Cargo networking
- [ ] 2.2 @regression (agent) Feed corrupt/truncated/unsafe prepared archives and missing or mismatched target inputs -> explicit build/preparation refusal, no fallback or partial executable

## 3. Safe extraction [critical]

- [ ] 3.1 @integration (agent) Exercise actual embedded extraction, concurrent first startup, cancellation, corrupt cache and adversarial archive cases -> only fully verified runtime activates and existing cache remains preserved on failure
- [ ] 3.2 @manual (agent) Inspect runtime provisioning dependencies and all install/build docs -> no runtime engine HTTP path and no separate end-user Dolt installation requirement

## 4. Repository completion [critical]

- [ ] 4.1 @integration (agent) Run mise run check and observe native hosted checks -> strict lint/tests/docs/cospec and at least 90 percent meaningful workspace coverage pass with recorded SHA
- [ ] 4.2 @manual (agent) Review canonical guidance and archive through cospec -> portability and bundled runtime contract durably reinforced without premature platform claims
