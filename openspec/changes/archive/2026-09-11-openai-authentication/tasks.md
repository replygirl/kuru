## 1. Correct OpenAI access

- [x] 1.1 Add the real CLI no-Codex authentication regression and observe its old-adapter failure, then implement native browser/device login, private token refresh/status/logout and verify actual local HTTP/filesystem lifecycle tests.
- [x] 1.2 Replace subscription subprocess requests with direct HTTP using the existing Responses conversion, wire existing CLI/config selections and remove the tool prerequisite; verify native status, model requests, API-key access, SSE/error bounds and existing continuation tests.

## 2. Complete the existing PR

- [x] 2.1 Update owning OpenAI/install/config documentation and verify commands/content without changing runtime behavior or release topology.
- [x] 2.2 Focused authentication checks and all normal hooks passed; final CI 34621868783 at db33c17 passed every static category and all five native targets. Windows passed 444 tests with 94.235428% workspace coverage and 60 platform tests with 91.378225% coverage; local coverage was 96.751509%. Installed/update acceptance, actual memory and process cleanup, ConPTY and shipping checks passed. Exact checkout/tree and raw evidence are recorded in /tmp/kuru-command-lock-ci-report.md.
- [x] 2.3 The user completed native browser login and a same-session live OpenAI conversation on September 11, recorded separately in verification 1.4. Final native and granular checks passed; strictly validate and archive the completed correction through cospec before the final PR commit.
