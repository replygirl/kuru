## 1. Native fixture invocation [critical]

- [x] 1.1 @regression (agent) execute shared artifact with an independent fixture alias as argv[0] -> old locator exited 1 with missing plan, test command exited 101; corrected locator passed exact response/transcript/completion assertions. Logs: /tmp/kuru-fixture-invocation-before.log and /tmp/kuru-fixture-invocation-after.log.
- [x] 1.2 @integration (agent) run full connector suite including concurrent hard-link isolation and final-owner cleanup -> 33 tests passed; stderr-discarded early-input failure retained started/error evidence. Strict package Clippy passed. Logs: /tmp/kuru-fixture-invocation-after.log and /tmp/kuru-fixture-invocation-lint.log.
- [x] 1.3 @integration (agent) run mise check -> MISE_LOCKED=1 mise run check exited 0 in 48.58 seconds, including format, lint, types, behavioral tests, docs, tooling, cospec and the 90% workspace coverage gate. Log: /tmp/kuru-fixture-invocation-check.log.

## 2. Hosted execution

- [~] 2.1 @runtime (agent) observe PR Linux and macOS checks after push -> defer: hosted checks require the archived change to be committed and pushed; both platforms must pass before merge, with links recorded in the PR.

## Observed trigger

`/tmp/kuru-frozen-tooling-push.log`: coverage connector suite 29 passed, 2 failed;
Codex offline assertion omitted actual error, RPC returned closed output. The
preceding local full gate passed. No exact scheduler/root-cause claim is made.

A separate local macOS probe preserved the invoked path in all 64 processes
(`/tmp/kuru-fixture-identity-probe.log`). The original intermittent failure was
not reproduced causally. The new regression proves invocation identity, and
startup/error sidecars plus panic-only bounded diagnostics retain evidence for
any future unexpected exit without changing production stderr handling.
