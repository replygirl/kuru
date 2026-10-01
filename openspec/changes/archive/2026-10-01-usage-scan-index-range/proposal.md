# Proposal

## Why

Every writable open runs two full owned-state scans of the usage ledger in
`establish` (`packages/kuru-memory/src/store/usage_ledger.rs`). Each scan pages
through `kuru.usage.v1/` with `LEFT(BINARY key, ?) = BINARY ? AND (? IS NULL OR
BINARY key > BINARY ?) ORDER BY BINARY key LIMIT 128`. On the pinned Dolt 2.3.5
the `BINARY(...)` wrapper on the column stops the planner from deriving a
primary-key range and `ORDER BY BINARY key` forces a sort, so every page plans
as `TopN` over a `Filter` over a full `Table` scan of `state`. Each page reads
the whole table and there are N/128 pages, so one scan is quadratic in the
number of usage rows. Measured on aged stores (unit 7 research, SQL only):
13 / 64 / 270 ms per page and 0.54 / 10.5 / 157.5 s per scan at 4,550 / 20,500
/ 80,075 rows, which matches the in-app open timeline's scan rows and exceeds
the 30 s open deadline on an old project.

The same shape appears in `session_index_page` (`/cost`) and in
`session_has_records_tx`, a full-table `COUNT(*) … FOR UPDATE` inside the
`mark_new_session` write transaction, against "keep transactions short".

## What Changes

- The two owned-state scans in `validate_branch` and the session index walk in
  `session()` page by a primary-key byte range: `key >= start AND key < end
  ORDER BY key LIMIT 128` for the first page and `key > after AND key < end
  ORDER BY key LIMIT 128` for later pages. Two fixed query strings avoid any
  bind-time `OR` folding. `start` is the prefix, `end` is the least byte string
  above every key that begins with it (the prefix with its last byte that is
  not 0xFF incremented and the rest dropped), computed once per scan. `after`
  is bound as bytes. Decode, canonical-key checks and error messages are
  unchanged.
- `mark_new_session`'s existence check becomes `SELECT 1 FROM state WHERE key
  >= ? AND key < ? LIMIT 1 FOR UPDATE` on the session's index range: an index
  range probe instead of a full-table `COUNT(*)`.
- Meaning is unchanged. `state.key` is `VARBINARY(1024)` with the binary
  character set, so `BINARY` was the identity on both comparison and order;
  a half-open byte range `[prefix, end)` on a binary column is exactly
  "starts with prefix", with no wildcard semantics.
- Every owned row is still decoded on every writable open; the
  validate-before-activation guarantee pinned by `usage-ledger-guarantee-tests`
  is untouched. Reducing the number of scans is a separate later change.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `memory-store-lifecycle`: ADDED requirement "Linear usage ledger
  validation" (design §5.2): owned-state validation reads each owned row a
  bounded number of times and pages by an exact byte range in byte-wise key
  order. Observable behaviour is otherwise identical.

## Benchmarks

| Metric | Before | After | How measured |
|---|---|---|---|
| One owned-state page, median of 10 (4,550 / 20,500 / 80,075 rows) | 13.09 / 64.35 / 270.39 ms | 0.40 / 0.42 / 0.49 ms | Unit 7 research §3.4: APFS clones of the unit 6b aged stores, Dolt 2.3.5 `sql-server`, SQL-level `PREPARE … EXECUTE` |
| One full owned-state walk, SQL only (same stores) | 0.537 / 10.501 / 157.505 s | 0.020 / 0.083 / 0.273 s | Same |
| One full owned-state walk through sqlx's binary protocol, bound as production binds it, median of 5 after (aged 1k / 5k / 20k: 4,550 / 20,500 / 80,075 rows) | 0.528 / 10.324 / 153.749 s (one walk) | 0.027 / 0.109 / 0.422 s | This change: an uncommitted harness in the crate's test module calling the old query and the production `owned_state_page` against fresh APFS clones of the unit 6b aged stores, pinned Dolt 2.3.5 `sql-server`, macOS arm64, debug test build, load average 4-19 |
| One page through sqlx, median over the walk | 13.73 / 62.82 / 241.83 ms | 0.71 / 0.67 / 0.66 ms | Same |
| Whole `validate_branch` (status, historical schema, walk, decode and key checks), median of 5, after | not measured | 0.087 / 0.361 / 1.379 s | Same; debug build (workspace code at opt-level 0), so the decode share is an upper bound for release |

## Impact

- `packages/kuru-memory/src/store/usage_ledger.rs`: the paging query strings,
  a private `KeyRange` with `prefix_upper_bound`, `owned_state_page`,
  `session_index_page`, `session_has_records_tx`, and the `after` cursor type
  in `validate_branch` and `session()` (bytes instead of UTF-8 text). The
  in-loop UTF-8 key checks remain. Tests module: range-bound unit tests, an
  ordering-equivalence test against the old query, and a plan test.
- No schema, on-disk, protocol or public API change; no interaction with older
  or newer binaries. `docs/memory.md` does not describe the scan, so no user
  documentation changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
