## 1. Correct OpenAI access

- [x] 1.1 Add the real CLI no-Codex authentication regression and observe its old-adapter failure, then implement native browser/device login, private token refresh/status/logout and verify actual local HTTP/filesystem lifecycle tests.
- [x] 1.2 Replace subscription subprocess requests with direct HTTP using the existing Responses conversion, wire existing CLI/config selections and remove the tool prerequisite; verify native status, model requests, API-key access, SSE/error bounds and existing continuation tests.

## 2. Complete the existing PR

- [x] 2.1 Update owning OpenAI/install/config documentation and verify commands/content without changing runtime behavior or release topology.
- [ ] 2.2 Run focused checks plus existing five-target native acceptance, resolve the remaining Windows CI failure and record passing granular hooks/CI with at least 90% coverage.
- [ ] 2.3 Complete the user-participating live OpenAI smoke check, record it separately, strictly validate and archive the completed correction before the final PR commit.
