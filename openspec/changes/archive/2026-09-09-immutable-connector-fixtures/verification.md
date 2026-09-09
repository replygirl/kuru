## 1. Native fixture execution [critical]

- [x] 1.1 @regression (agent) immutable fixture publication and isolated concurrent plans -> before-fix test failed on separate executable inodes (exit 101); both corrected regressions passed on macOS, including concurrent exact transcripts, read-only executable sharing, surviving-owner execution, last-owner cleanup and cache rebuilding. This proves the publication invariant, not a local reproduction of Linux's scheduler-dependent ETXTBSY. Logs: /tmp/kuru-immutable-fixtures-before.log and /tmp/kuru-immutable-fixtures-after.log.
- [x] 1.2 @integration (agent) connector test suite under normal concurrency -> package mise task passed all 31 tests with loopback access, including model catalog, completion, authentication, MCP and RPC. An initial sandboxed attempt failed only loopback binds with EPERM; the authorized run passed without code changes or launch retries.
- [x] 1.3 @integration (agent) full mise run check -> passed with 217 Rust tests and 97.55% line coverage (8863/9086), including format, strict Clippy, docs, tooling and managed cospec drift checks. Log: /tmp/kuru-immutable-fixtures-check.log. Reviewed README and install guide preserve authenticated private-repository installation while removing no-release-yet claims.

## 2. Hosted publication

- [~] 2.1 @runtime (agent) Linux/macOS CI and authorized first release -> defer: requires the archived change's final commit and hosted run; results will be linked in PR and release completion evidence.
