## 1. Correct native acknowledgment timing

- [x] 1.1 Preserve the observed pre-fix installed update failure, add a real slow-publication helper regression and separate the bounded publication wait from startup/frame limits. CI 34607501091 completed actual Windows embedded install/update in 134.46 seconds and passed all 12 native updater tests, including publication held beyond ten seconds.
- [x] 1.2 Verify actual IPC timeout, truncated/oversized frames and existing exact-image/recovery behavior without weakening their checks. CI 34612890798 passed all 12 native updater tests, including the revised retained-root and cleanup assertions from e080f84; Windows log lines 1112–1116.
- [x] 1.3 Run owning static checks and native Windows CI, record installed update acceptance and prepare the completed record for strict validation/archive before the final branch commit. All eight normal hooks and all hosted jobs passed at ca8e38a; native source-installed Windows offline install/update and shipping DLL inspection passed, with 94.235428% workspace coverage.

The subsequent test-only cleanup and retained-root correction in e080f84 passed
natively in CI 34612890798. Its log is
/tmp/kuru-shell-lookup-ci-job-103307525914.log. The independent shell diagnostic
record still requires plain-source acceptance; it does not change this updater
implementation or the observed native updater results.
