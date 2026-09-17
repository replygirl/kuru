## 1. Private-stage delete completion [critical]

- [~] 1.1 @regression (agent) run Windows native cleanup with a held nested stage directory released after the first recoverable removal result -> defer: Windows native execution is pending; the prior exact-head fixture failed on typed uncertain delete-pending completion.
- [~] 1.2 @regression (agent) run Windows native OS145 outer cleanup after private-child deletion -> defer: Windows native execution is pending; the prior exact-head P6 application fixture observed OS145 after publication.
- [x] 1.3 @integration (agent) run host successful staged activation -> package-owned `provision::native_tests::successful_activation_removes_its_disposable_stage` passed 1/1 on macOS.

## 2. Static and review

- [x] 2.1 @unit (agent) run memory all-target/all-feature typecheck, lint, formatting, and diff checks -> all passed on the host with the shared verified bundle mirror.
- [x] 2.2 @manual (agent) independently inspect typed uncertain/rejected branching, exact identities, no-mutation pending waits, and first-cause preservation -> two independent reviews cleared the final source; Windows-native execution remains pending.
