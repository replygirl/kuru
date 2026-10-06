# Tasks

## 1. Fake peer preparation

- [x] 1.1 Extend the existing std-only compile/cache path in `packages/kuru-connectors/src/test_support.rs` to Windows with explicit coverage opt-out, platform-owned compiler launch and checked full completion; preserve immutable snapshots and inspect all post-spawn error paths.
- [x] 1.2 Extend the existing Windows snapshot regression to execute the compiled fake peer with an isolated `LLVM_PROFILE_FILE`, verify protocol/snapshot/alias/cleanup assertions and no raw profile, and run the relevant host protocol-failure and fixture ownership selections; distinguish local results from pending native Windows execution.

## 2. Completion

- [x] 2.1 Update `docs/development.md` with the fake-peer-only boundary and qualified historical attribution, run affected host/Windows lint, types, docs, format and managed validation, complete source review and strict/apply context verification before actual archive and normal commit.

## Evidence

Before implementation: PR239 Windows shard 1 completed all 288 tests before strict profile export rejected PID 6372's 1,232-byte profile. Its visible 64-byte header is nonempty/version 11 with 9 data records and 54 counters. The matching 1,784-byte sibling's timestamp precedes the later recorded PID 7436 memory spawn, so that signature-to-memory mapping is polluted by PID reuse. The corruption timestamp falls within the connector RPC failure fixture's run. A pinned Windows-target IR probe of the std-only peer shows 9 functions; pinned LLVM version 11 has a 152-byte header and 72-byte data records, and an uncompressed names blob from the same source is 549 bytes plus padding. This is strong fixture evidence, not exact historical writer or truncation-section proof. The pinned compiler's coverage annotation is unstable; the supported standalone compile flag avoids a compiler feature or pin change. The pinned cargo-llvm-cov default report already excludes workspace `tests/fixtures` sources.

The standalone compiler wait has no separate execution deadline, matching the existing Unix fixture compiler. It retains the actual compiler owner until completion; the owning runner's existing overall deadline remains authoritative. No elapsed interval establishes completion and no new timeout or process framework is added.

Implementation and director source review are complete. Only the fake peer is compiled with `instrument-coverage=no`; every actual application child retains instrumentation and its profile destination. The Windows compiler uses the existing platform child/Job owner with null/file stdio, awaits actual whole-Job completion even after a wait-observation error, and reports diagnostics or snapshot failures only afterward. Existing immutable snapshot, private cache and alias checks remain intact. Existing MCP forced termination and protocol deadlines remain unchanged. The no-profile snapshot assertion observes normal RPC close; separate existing protocol-failure cases cover forced cleanup.

Observed local checks:

- Task 72577 exited 0: owning connector host lint, Windows-target lint and all-target/all-feature typecheck.
- Task 46898 exited 0: exact host selections `rpc_bounds_protocol_failures_timeout_and_oversized_dispatch`, `stdio_fixtures_share_executable_and_isolate_concurrent_plans`, and `final_fixture_owner_cleans_artifact_and_cache_can_rebuild` passed 3/3 (1.90 s test body). The Windows-only snapshot/no-profile filter selected zero tests on this host.
- Task 30090 exited 0: documentation build/content/link checks, Rust/TOML/fixture formatting and managed-file validation.
- Initial strict validation and apply gates exited 0 (1843e9 and 1f2b6c), and all three returned context files were read before implementation. Final strict/apply/context verification is repeated before archive.

Native Windows execution of the compiled-peer snapshot/no-profile assertion and hosted strict coverage export remain pending CI. These local results do not establish that the historical corruption is resolved. No blind coverage retry, profile filtering, threshold reduction or application-module exclusion is part of this change.
