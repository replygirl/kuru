# Proposal

## Why

`UsageLedger::session` re-derives a session's usage on every call by folding
its immutable invocation records, which is deliberate: no mutable session total
is stored, so retries and uncertain receipts cannot double count. But it reads
each record with its own `read_state` statement, one per indexed invocation, so
one call issues 1 marker read + P index page reads + 1 trailing empty-page read
+ N record reads = P + 2 + N statements. The TUI calls `session_usage()` after
every settled turn (`apps/kuru-tui/src/ui.rs:2070`), and the service
(`packages/kuru-memory/src/service/rpc.rs`) and `/cost` reach the same fold, so
the per-turn cost grows linearly with session length and quadratically over a
session (fixed-wait audit 2026-10-02, §3D rank 1, unit U5 ledger half).

## What Changes

- `session()` reads each index page's record keys with one keyed statement
  (`SELECT key, value FROM state WHERE key IN (?, …)`, at most `PAGE_SIZE` keys,
  under its own `QUERY_TIMEOUT` budget and deadline context), then walks the
  page in its existing key order looking each record up. Fold order, the
  decoders and every `ensure!` text (`usage session index key does not match
  its record`, `usage session index references a missing invocation`, `usage
  session index references a mismatched invocation`) are unchanged.
- No stored aggregate, no new constants or budgets, no schema or API change.
- Test-only: a task-scoped statement counter in the pool-level read helpers
  `session()` uses, and a test that asserts the exact count per call.

## Benchmarks

Metric: SQL statements issued by one `session()` call, counted by a
`#[cfg(test)]` `tokio::task_local!` counter incremented once in each pool-level
read helper `session()` calls (`read_marker`, `session_index_page`,
`read_state`, and the new keyed read), each of which issues exactly one
statement. Measured by `session_reads_each_index_page_with_one_keyed_record_read`
over a session of N = 300 invocations (P = 3 index pages of `PAGE_SIZE` 128),
debug test build, pinned Dolt, macOS arm64.

| Build | Formula | Statements per `session()` (N = 300, P = 3) |
|---|---|---|
| Before (`origin/main` 7c9581b0) | P + 2 + N | 305 (observed) |
| After | 2P + 2 | 8 (expected; recorded after the fix) |

The after cost depends only on the page count: 1 marker read + P index pages +
1 trailing empty page + P keyed record reads. It is still linear in session
length, but at two statements per 128 invocations instead of one per
invocation; collapsing the page and record reads into one self-join (P + 2) is
a different change and is not made here.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Behavior is identical; no performance SLO becomes a requirement.

## Impact

- `packages/kuru-memory/src/store/usage_ledger.rs`: `UsageLedger::session`, a
  new file-local keyed read helper, the test-only counter and its test.
- No public API, schema, migration, configuration or documentation change.
  `packages/kuru-memory/src/migration.rs` (the other half of audit unit U5) is
  separate work.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
