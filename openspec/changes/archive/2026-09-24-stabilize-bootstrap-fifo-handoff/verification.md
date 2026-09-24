## 1. Immediate producer failure terminates [critical]

- [x] 1.1 @regression (agent) repeatedly run the real macOS FIFO handoff with a missing local asset -> pre-fix writer-first runs stranded `head` with the producer gone at iterations 112 and 70; corrected reader-first completed 1,000/1,000
- [x] 1.2 @e2e (agent) run the bootstrap missing/corrupt/malformed input regression through the shipped installer -> corrected bootstrap test passed 17/17, including specific failure reporting and unchanged-install assertions

## 2. Bounded successful and adversarial streams

- [x] 2.1 @integration (agent) run normal local package installation through the shipped installer -> corrected bootstrap test passed actual package and explicit-destination installation
- [x] 2.2 @integration (agent) run oversized and held-open transport cases through the shipped installer -> corrected bootstrap test passed manifest/archive overflow, held-open producer, signal cleanup and unchanged-install cases

## 3. Repository acceptance

- [x] 3.1 @integration (agent) run delivery shell lint and focused bootstrap-install checks -> shellcheck, repository format, archive 15/15, bootstrap 17/17 and strict cospec validation passed; independent Config review is clear
- [x] 3.2 @runtime (agent) run the exact #71 head on macOS and supported native platforms -> exact-head b200240 CI 36010519061 passed macOS 14 native coverage, macOS 15 Intel build, Ubuntu native coverage, Windows native platform/application/delivery/install checks and its 92.46% aggregate coverage report without a bootstrap FIFO timeout
