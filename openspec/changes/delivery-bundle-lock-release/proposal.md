## Why

`kuru-delivery` bundle preparation holds a stable advisory lock (`.bundle.lock`, `Directory::lock` in `packages/kuru-delivery/src/bundle.rs`) and releases it only by closing its descriptor when the returned `File` drops. On Unix a `flock` belongs to the open file description and is released on close only when every descriptor referring to that description is closed; a child that any thread of the process is spawning holds a copy of every descriptor from its fork/`posix_spawn` until its exec closes the close-on-exec ones. A preparation that returns or is cancelled while another thread spawns a child therefore leaves the lock held past its owner's drop, so an immediate contender (another preparation, or a test's release assertion) observes `WouldBlock`. On Windows, closing a handle with outstanding byte-range locks unlocks them at a time the system documents as dependent on available resources, so release-by-close is not prompt there either.

It surfaced as flaky-test catalogue items 7 and 9 (`tmp/roadmap/phase2-handoff-2026-09-26-sources/flakes/flaky-tests.md`; this change's 7a): `bundle::recovery_tests::cancellation_after_dropping_error_headers_releases_stage_and_stable_lock` failed at `assert_clean` → `lock.try_lock()` = `WouldBlock` in PR #118 run 36294245109 job 108550021448 (ubuntu-latest partition 7) and in run 36304624151 on both attempts, jobs 108578990024 and 108582217193 (macos-latest partition 3, deterministic for that selection). Locally on macOS the `bundle::` tests failed 13/20 at default test threads on 9def614b and 0/10 at `--test-threads=1`. #118 (archived `2026-09-26-coverage-partitions`) classified it as a test isolation defect and re-executed ten release-asserting tests alone in a child test process (`isolate_lock_release!`). That hides the product fault: the tests are right to expect prompt release, and the same race applies to any process that spawns while it prepares a bundle.

## What Changes

- `Directory::lock` returns a guard that holds the checked lock file and explicitly unlocks it (`File::unlock`: `flock(LOCK_UN)` on Unix, `UnlockFile` on Windows) before its descriptor closes, on every exit path including error returns, panics and future cancellation. Release no longer depends on other descriptors of the same open file description.
- The guard dereferences to the held `File`, so identity verification (`Directory::verify`) is unchanged. Drop order in `prepare_asset_with_policy` is unchanged: the private stage (declared after the lock) is removed first, then the stable lock is released.
- The Windows installation lease (`update.rs` `lock`/`installation_guard`) uses the same guard, so both platforms share one explicit-release path.
- Test-side helpers that take and release the same lock on fresh handles (`assert_clean`, `retained_lock`) also unlock explicitly.
- The test-only `isolate_lock_release!`/`delegated_to_lock_child` re-execution introduced by #118 is removed: its stated rationale (release by close can be defeated by a concurrent spawn) is eliminated by explicit release, and it serialized the release-asserting tests.
- A deterministic regression test models an inherited duplicate (`try_clone` of the held lock file), drops the lock the way the product does, and requires a fresh handle to take the lock immediately.
- Not changed: lock acquisition, its bounded wait (`LOCK_TIMEOUT`, 25 ms polling), identity/replacement checks, stage creation/removal, download retries and deadlines, and any test assertion or timeout. No retry is added.
- Not changed here (recorded follow-ons): the same release-by-close pattern in `kuru-memory`, `kuru-tui` and `kuru-connectors` (see Impact).

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living specs do not describe the lock release mechanism; only the implementation was wrong.

## Impact

- `packages/kuru-delivery/src/lease.rs` (new): `HeldLock` explicit-release guard.
- `packages/kuru-delivery/src/bundle.rs`: `Directory::lock` returns `HeldLock`; test helpers unlock explicitly; isolation delegation removed; regression test.
- `packages/kuru-delivery/src/bundle/recovery_tests.rs`: `retained_lock` unlocks explicitly; `isolate_lock_release!` uses removed.
- `packages/kuru-delivery/src/update.rs` (Windows only): `lock`/`installation_guard` return `HeldLock`; `archive.rs` discards it as `_lease` unchanged.
- `packages/kuru-delivery/src/lib.rs`: module declaration.
- Audit of other advisory locks released by close (follow-ons, not edited here; `kuru-memory` has another change in flight): `kuru-memory` `provision.rs` cache lock, `server.rs` lifecycle lease, `service.rs`, `store.rs` writer lease; `kuru-tui` `trust.rs` lock and `cli.rs` lease; `kuru-connectors` `auth/store.rs`, `mcp_cache.rs`, `mcp_credentials.rs`. `kuru-memory` and `kuru-tui` carry test-only `spawn_gate.rs` mitigations for the same mechanism, which do not fix the product path. Promoting the guard to `kuru-platform` is the suggested vehicle.
- No public API, configuration, documentation or schema change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
