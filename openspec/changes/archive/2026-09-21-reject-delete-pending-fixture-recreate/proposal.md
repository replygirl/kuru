## Why

`provision::native_tests::real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution` failed on main's Windows memory-runtime shard (run 35584395348): `new_private_file(&binary).unwrap()` panicked with "Access is denied (os error 5)" immediately after `remove_fixture_binary(&binary, identity).unwrap()` had already returned `Ok(())`. The fixture deletes a `dolt.exe` cache binary the warm probes just executed; that just-executed image's name can stay delete-pending for a moment after the checked removal reports success, because the OS tears down the executed image section asynchronously. `remove_fixture_binary` returned as soon as the delete call itself succeeded, so the very next `new_private_file` create-new at the same name raced that teardown and was refused before the pending delete completed. `concurrent_cold_windows_provision_publishes_one_verified_native_identity`'s sibling call site at the corrupt-cache fixture shares the exact same shape and is exposed to the same race.

## What Changes

- `remove_fixture_binary` no longer returns as soon as the removal call succeeds: it now waits, bounded by the same 2-second window it already uses for the removal retry itself, until the removed name is genuinely absent (confirmed only by `NotFound`; a successful or still-denied query is never treated as proof of absence) before returning control to its caller.
- The wait is folded into `remove_fixture_binary` itself rather than added at each call site, so both existing callers benefit without duplicating the pattern.
- On exhaustion the new wait reports the path, the reconcile attempt count and the elapsed window, mirroring the exhaustion-message style already established for the bounded stage-cleanup recovery in `files.rs`.
- No new magic numbers: the removal retry and the absence wait now share one named 2-second/20-millisecond pair of constants instead of each embedding its own literal `Duration::from_secs(2)` / `Duration::from_millis(20)`.
- No other kuru-memory test fixture recreates or reopens a name it just removed under the same just-executed-image shape; the two `remove_fixture_binary` call sites are the only sites affected.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/provision/native_tests.rs`: `remove_fixture_binary` (Windows-only fixture helper) and its two call sites gain a bounded post-removal absence wait via a new `wait_for_fixture_binary_absence` helper, plus three unix-runnable unit tests for that helper. No platform API, public setting, deletion policy, recovery predicate, or production code path changes — this is test-only.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
