# Tasks

## 1. Owned-row refusal tests (`packages/kuru-memory/src/store/usage_ledger.rs`)

- [ ] 1.1 Add a seeding helper that writes one scripted session through the ledger's real write path, and a planting helper that writes a foreign commit on the usage branch through a released server, and verify both by a final writable reopen that reads the seeded usage back
- [ ] 1.2 Add the malformed-value test (not JSON, unknown field, failed field validation, for each of the four classes at canonical keys) and verify that every case refuses with its decoder's message while a read-only open succeeds
- [ ] 1.3 Add the non-canonical-key test (a real value of each class under a key not derived from it) and verify that every case refuses with its key check's message while a read-only open succeeds
- [ ] 1.4 Add the foreign-key test (unknown class, bare prefix, non-UTF-8 key under the prefix) and verify the unrecognized-key and non-UTF-8 messages while a read-only open succeeds
- [ ] 1.5 Add the boundary-key test (`kuru.usage.v1`, `kuru.usage.v1.x`, `kuru.usage.v10x` not refused; `kuru.usage.v1/` refused) and verify the seeded usage still reads back
- [ ] 1.6 Add the pool-not-published test (direct `establish` over a planted row) and verify that the error is the scan's and that `usage_pool` stays unpublished

## 2. Evidence

- [ ] 2.1 Stub the owned-state scan out locally (an early `return Ok(())` before its loop, which disables it at both call sites), run the new tests, record which fail and how, revert, and verify that the diff touches only the tests module
- [ ] 2.2 Run `mise run //packages/kuru-memory:test`, `mise run format:check`, `mise run lint` (host and Windows targets) and `mise run typecheck`, and record the results with the new tests' added time
