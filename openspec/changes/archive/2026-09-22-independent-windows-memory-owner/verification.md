## 1. Independent Windows owner after starter exit [critical]

- [~] 1.1 @regression (agent) launch a native Windows starter inside a kill-on-close Job that permits breakaway, attach a second client, exit the starter and release its Job -> defer: the new platform and memory fixtures cover this exact process sequence, same-generation append/history, endpoint retirement and owner-lock release, but native execution awaits exact-head hosted Windows CI after the PR push; cross-target compilation is not execution evidence
- [~] 1.2 @integration (agent) launch the same starter inside a Job that denies breakaway -> defer: the new native memory fixture asserts an explicit containment diagnostic, absent endpoint and free owner lock, while the platform fixture asserts the independent child never starts; native execution awaits exact-head hosted Windows CI

## 2. Narrow platform behavior

- [~] 2.1 @regression (agent) run existing native Windows `OwnedJob` and `TrustedSupervisor` lifetime fixtures beside the new independent-service cases -> defer: unchanged modes remain in the owning platform test task, but native Windows CI has not run this final head
- [~] 2.2 @regression (agent) run native macOS/Linux memory owner fixtures -> defer: macOS platform native suite passed after restricted sandbox EPERM was resolved with native socket access; `mise run //packages/kuru-memory:test` passed 188 library tests and all integration targets (6+5+12+1 cases) in 233.77s with its package-owned supervisor snapshot and two-thread policy; Linux awaits hosted CI

## 3. Repository gates

- [~] 3.1 @regression (agent) run relevant format, lint, typecheck, strict Cospec and normal pre-push checks -> defer: macOS memory/platform package typecheck and lint, Windows platform cross-target check, Rust format, `git diff --check`, strict Cospec validate/apply, macOS platform native tests, and the full macOS memory owning suite passed locally; the normal pre-push hook and hosted Windows run follow the archived branch commit and push. The first platform test run encountered sandbox EPERM on Unix sockets; the native-access rerun passed all targets and is the behavioral result

## Hosted correction evidence, 2026-09-22

The initial PR #56 head `d653415f78a6c8c8ec0d64b6c6be84735c7039a6` passed its normal pre-push hook, including 742.53 seconds of combined coverage. Its hosted Windows coverage shards failed during common `kuru-memory` lib-test compilation before any new Windows service fixture ran: E0308 at `service.rs:1206` returned the package's private `test_support::TempDir` from a helper declared as `tempfile::TempDir`. The earlier local Windows cross-target check covered `kuru-platform` only and did not compile that conditional memory fixture. The helper now declares the actual private fixture type, and ConfigSol independently reviewed the exact one-line correction clear. An attempted macOS-hosted `cargo check -p kuru-memory --target x86_64-pc-windows-msvc --all-targets --all-features --locked` stopped before Rust memory compilation because this host lacks Windows C/SDK headers: bundled `libsqlite3-sys` failed through `mm_malloc.h` with missing `stdlib.h`. It is not a Windows fixture compile result. Corrected exact-head native Windows CI remains pending; no Windows behavior pass is claimed from the failed jobs or this cross-target attempt.
