## 1. Confirmed Unix shell completion releases its reservation [critical]

- [x] 1.1 @regression (agent) synchronously inspect the registry when the oneshot receiver wakes after a confirmed shell result -> pre-fix report-before-removal observed 2 owners and failed; corrected `complete` observed 1 owner while ID 42 remained, 2026-09-13.
- [x] 1.2 @integration (agent) run the existing Unix retained-shell suite -> 14 Unix-shell tests passed, including natural completion, pre-spawn failure, caller loss, and retained unconfirmed cleanup, 2026-09-13.

## 2. Scoped validation

- [x] 2.1 @unit (agent) run the connector's focused Unix shell test target -> isolated-target typecheck passed; the regression passed after the correction; `mise run format:rust` and `git diff --check` passed, 2026-09-13.
- [~] 2.2 @manual (agent) Windows behavior -> defer: this Unix-only correction does not establish a new Windows native observation.
