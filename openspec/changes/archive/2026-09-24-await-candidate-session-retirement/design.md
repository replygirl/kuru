## Context

`Server::retire_pool` awaits SQLx pool closure, then `transition_candidate` immediately calls `DOLT_BRANCH('-m')`. Pool closure does not acknowledge Dolt's server-side session teardown. Exact #71 macOS coverage observed Dolt refuse a rename because another session still used the source branch. A local four-connection probe observed prompt teardown, so it did not reproduce that timing. The earlier Windows `StorageFailed` remains unclassified.

## Goals / Non-Goals

**Goals:** Wait for authoritative server-side source-branch idleness before a checked candidate rename, with bounded refusal before mutation and no new source connection admitted during the transition.

**Non-Goals:** Force or retry a rename, change public RPC or storage schema, attribute the earlier Windows failure without its own evidence, or serialize unrelated branches behind a global lock.

## Decisions

- Fence `Server::pool(source)` and candidate retirement/rename with one source-branch async admission guard. The transition retains that guard through its existing rename result and reconciliation/ref check, then releases it. A short-held map lookup may find the guard; the potentially long server wait does not hold the global pool-map mutex. Other branches remain independent.
- After closing the source pool, query pinned Dolt's `information_schema.processlist` from the existing main pool for exact, binary-equal `DB = kuru/<source>`. Wait until the count is zero under the existing bounded pre-dispatch deadline, before calling `DOLT_BRANCH`. Kuru's connection setup selects that database and does not switch databases on a live candidate connection; the native fixture must verify that exact association. This branch-qualified check covers Kuru-created and intentionally external source sessions without a growing session-ID registry.
- A query error or deadline exhaustion is a pre-rename refusal: preserve the exact source and status refs, do not force, kill, replay, or issue the rename. Once the rename is sent, keep the existing uncertain-write reconciliation and recovery classification. Preserve the checked source/head and write/ref reservation around the operation.

## Risks / Trade-offs

- An intentionally held external session can delay a transition until the deadline. The failure is bounded and leaves the ref unchanged; a later fresh checked operation may proceed after that session ends.
- Database attribution could be wrong if a connection switches databases after admission. Review the pinned connection paths and prove `DATABASE()` against processlist `DB` in the real-Dolt fixture; do not infer the guarantee from current online documentation alone.
- A wait guard could deadlock with a path that reopens the source pool during reconciliation. Keep the guard's call graph explicit and release it before any later clean-check that needs source admission, without opening a gap before the rename/ref result is classified.

## Integration contract

`kuru-memory` alone owns the source-branch SQLx pool and the Dolt `DOLT_BRANCH` call. The comparison uses the existing branch name validated by Kuru and the database name used by its pinned Dolt connection; `BINARY` equality avoids collation aliases. The main-pool processlist query is an observation of server sessions, not authority to terminate them. Native tests must exercise the pinned Dolt version, exact database/connection IDs, a held source session, concurrent source admission, timeout/no-effect, and a successful single transition after teardown.
