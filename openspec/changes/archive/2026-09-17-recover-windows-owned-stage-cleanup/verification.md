## 1. Windows owned-stage recovery [critical]

- [~] 1.1 @regression (agent) run Windows controlled held-handle release, persistent-holder exhaustion, replacement refusal, and same-identity corrupt-cache invalidation -> defer: these fixtures are Windows-only; corrected native CI is pending. They fail before the change because close either reports immediately or the fixture raw deletion receives OS32.
- [x] 1.2 @integration (agent) run a successful real staged activation -> package-owned `provision::native_tests::successful_activation_removes_its_disposable_stage` filter passed 1/1 on macOS, proving the regular disposable-stage cleanup path remains successful.

## 2. Static and review

- [x] 2.1 @unit (agent) run memory typecheck, lint, formatting, and diff checks -> `//packages/kuru-memory:typecheck` and `:lint`, `cargo fmt --all -- --check`, and `git diff --check` passed on the host.
- [x] 2.2 @manual (agent) obtain independent review of private ownership, OS32 reconciliation, error preservation, and fixture-only corruption scope -> independent reviews cleared the final source; Windows-native execution remains pending.
