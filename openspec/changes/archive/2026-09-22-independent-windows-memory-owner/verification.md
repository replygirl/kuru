## 1. Independent Windows owner after starter exit [critical]

- [~] 1.1 @regression (agent) launch a native Windows starter inside a kill-on-close Job that permits breakaway, attach a second client, exit the starter and release its Job -> defer: the new platform and memory fixtures cover this exact process sequence, same-generation append/history, endpoint retirement and owner-lock release, but native execution awaits exact-head hosted Windows CI after the PR push; cross-target compilation is not execution evidence
- [~] 1.2 @integration (agent) launch the same starter inside a Job that denies breakaway -> defer: the new native memory fixture asserts an explicit containment diagnostic, absent endpoint and free owner lock, while the platform fixture asserts the independent child never starts; native execution awaits exact-head hosted Windows CI

## 2. Narrow platform behavior

- [~] 2.1 @regression (agent) run existing native Windows `OwnedJob` and `TrustedSupervisor` lifetime fixtures beside the new independent-service cases -> defer: unchanged modes remain in the owning platform test task, but native Windows CI has not run this final head
- [~] 2.2 @regression (agent) run native macOS/Linux memory owner fixtures -> defer: macOS platform native suite passed after restricted sandbox EPERM was resolved with native socket access; `mise run //packages/kuru-memory:test` passed 188 library tests and all integration targets (6+5+12+1 cases) in 233.77s with its package-owned supervisor snapshot and two-thread policy; Linux awaits hosted CI

## 3. Repository gates

- [~] 3.1 @regression (agent) run relevant format, lint, typecheck, strict Cospec and normal pre-push checks -> defer: macOS memory/platform package typecheck and lint, Windows platform cross-target check, Rust format, `git diff --check`, strict Cospec validate/apply, macOS platform native tests, and the full macOS memory owning suite passed locally; the normal pre-push hook and hosted Windows run follow the archived branch commit and push. The first platform test run encountered sandbox EPERM on Unix sockets; the native-access rerun passed all targets and is the behavioral result
