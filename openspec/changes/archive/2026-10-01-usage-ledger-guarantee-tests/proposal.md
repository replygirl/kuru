# Proposal

## Why

`memory-store-lifecycle` ("Permanent operational usage branch lifecycle") and
`context-usage-accounting` ("Invalid operational branch") already require a
writable open to validate the usage ledger's owned contract before it allows
provider dispatch. In code that means every row under `kuru.usage.v1/` must
decode as its typed value and sit under the canonical key derived from that
value before `establish` publishes the usage pool. No test plants a bad owned
row and reopens, so nothing pins that guarantee before the paging query and
the validation path are reworked for open-time cost.

## What Changes

- `packages/kuru-memory/src/store/usage_ledger.rs` (tests module only).
  Valid rows come from the ledger's real write path (`mark_new_session`,
  `admit`, `observe`, `settle`). Each fault is then planted by a foreign
  commit on the usage branch through a released server (raw SQL plus
  `DOLT_COMMIT`). Every refusal is asserted by its specific scan message,
  not by `is_err()`. The tests cover:
  - **Malformed values.** For each of the four owned classes (session
    marker, invocation record, observation, session index), a value at its
    canonical key that is not JSON, that carries an unknown field, or that
    fails field validation. The writable reopen refuses.
  - **Non-canonical keys.** A real value of each class copied under a key
    that is not the one derived from it. The writable reopen refuses.
  - **Foreign keys under the prefix.** An unknown class
    (`kuru.usage.v1/future/x`), the bare prefix `kuru.usage.v1/`, and a
    non-UTF-8 key under the prefix. The writable reopen refuses.
  - **Read-only opens.** In every refused case, a read-only open of the same
    store still succeeds and reads main.
  - **Boundary keys.** `kuru.usage.v1`, `kuru.usage.v1.x` and
    `kuru.usage.v10x` sit outside the prefix and are not refused, and the
    seeded usage still reads back. A row at exactly `kuru.usage.v1/` is then
    refused, which pins the range edge and keeps the test discriminating.
  - **Pool not published.** A direct `establish` over a planted bad row
    fails with the scan's error. It leaves `usage_pool` unpublished, so
    `usage_ledger()` stays unavailable.
- After each faulty case, the planted commit is reset away. A final writable
  reopen must succeed and read the seeded usage back, which shows that each
  refusal came from its plant.

## Impact

- Test-only. No product code, spec text or on-disk format changes.
- Adds a few kuru-memory test functions that each start several Dolt
  servers in sequence. The added time is recorded in `tasks.md`.
- This change exists so that the later range-paging and validation-record
  changes have a pinned guarantee to preserve.
