# Verification

Authored before either owner's implementation. Local and hosted results remain
distinct; ordinary owning mise tasks prepare verified real-Dolt fixtures.

## 1. Immutable complete topology reads [critical]

- [x] 1.1 @integration (agent) real Local/managed cuts with more than 256 reports, archived identities, extra slash/long identities and concurrent membership/report updates between pages -> memory cuts passed 3/3 and runtime topology fixtures passed 7/7; every returned membership/session/report belongs to one captured revision and the complete public map remains intact
- [x] 1.2 @integration (agent) live/candidate cuts, crossed cursors and handles, explicit close, attachment disconnect and owner restart -> isolation retained, foreign/closed/stale handles refused and owned resources released without closing another reader
- [x] 1.3 @integration (agent) small coherent fast path and byte/count fallback -> same full Topology as immutable-cut loading, without a graph cap or invented report index

## 2. Compatible staged migration [critical]

- [x] 2.1 @integration (agent) cold schema-9 then-latest multi-mode legacy blobs including part/relationship/archived/extra reports and project focus -> original migration fixtures passed 2/2; the added equal-member/report version-7 case passed in the final scoped 8/8, absent destinations start at zero, raw fields and original bytes remain unchanged and focus remains unassigned
- [x] 2.2 @integration (agent) malformed topology and conflicting membership/hash-report destinations -> the expanded refusal fixture passed in the affected selection, with entire-attempt rollback and no partial live publication or schema advance; old-registry refusal also passed
- [x] 2.3 @integration (agent) historical schema-9 candidate, reopen, export and migration publication fixtures -> affected history/export/publication checks passed, with five exact stale current-schema assertions corrected and passing in the final scoped rerun; historical assertions remain intact

## 3. Runtime delta ownership and exact publication [critical]

- [x] 3.1 @integration (agent) stale membership seed/relate/undo plus independent reports and sessions -> runtime topology fixtures passed 7/7: membership CAS retains the winner, report writes retain last-report policy and unrelated state/focus survives
- [x] 3.2 @integration (agent) real harness mode/model/effort/focus/turn checkpoints and large retained graph -> focused 7/7 and existing regressions passed after the two diagnosed cases were corrected and rerun; single-driver behavior and public Topology remain preserved, with no legacy or membership rewrites during ordinary updates
- [x] 3.3 @integration (agent) candidate dream/legacy undo and accepted lost replies -> focused and existing publication/dream/undo fixtures passed; candidate membership/undo/log stays isolated with no session/report writes, exact recovery precedes publication and newer conversations survive
- [x] 3.4 @eval (agent) controlled-provider dream/tool traces in existing mode-consolidation and canonical-dream fixtures -> the existing seven-module owning regression passed the ordered-subset/proposal-limit/typed-usage trace; the corrected canonical-dream fixture passed in the focused 2/2 rerun, preserving exact instruction bytes through promotion/reload/undo. This is an offline behavioral trace with fixture proposals/providers, not a live model-quality or broader peer-behavior evaluation.

## 4. Exact bounds and delivery checks

- [x] 4.1 @integration (agent) escaped payload and exact service-envelope boundary, membership above 16 MiB and one larger read row -> conditional fixtures passed 5/5 and read-cut fixtures 3/3: legal large records succeed, envelope overflow refuses before mutation/receipt and bounded pages make progress
- [x] 4.2 @unit (agent) exhaustive read/write/receipt contracts and generated wire pin -> golden regeneration passed 1/1, the normal pin and exhaustive contracts passed in the affected selection and final encoding-parity fixture passed in scoped 8/8; all new calls are classified and protocol 1.9 advances with its golden
- [x] 4.3 @integration (agent) owning memory/runtime fixtures, core all-target compilation, relevant regressions, host/Windows lint, formatting, docs and strict/managed Cospec -> focused and affected results recorded below; final common host and Windows lint, formatting, full docs build/content check, managed no-drift and strict validation all exited 0. Independent owning reviews cleared source; hosted/native/90% coverage acceptance remains unrun locally and is reported separately after delivery

## Observed local evidence

All live commands below used process-local `ulimit -n 4096`, the owning mise
task's prepared supervisor, and separate verified caches:
`/private/tmp/kuru-phase2-topology-state-memory-cache` and
`/private/tmp/kuru-phase2-topology-state-runtime-cache`. No coverage run or
hosted/native acceptance has been claimed.

- `mise run //packages/kuru-memory:test -- state_read_cut`: terminal exit 0,
  3/3 passed, 7.83 s. Real committed paging, concurrent report/membership updates,
  live/candidate isolation, oversized SQL suppression, negative versions, foreign
  cursor/attachment, cancellation, interrupted close, peer survival and restart.
  The first attempt exited 101 on five test-only compile errors; after correction
  all three ran and passed. Subsequent cleanup scopes use the existing helper and
  explicit direct-server registration; their structural guard passed in the
  affected regression selection.
- `mise run //packages/kuru-memory:test -- topology_state_v9`: terminal exit 0,
  2/2 passed, 7.44 s. All-mode then-latest byte-preserving migration and malformed
  or membership-conflicting atomic refusal. The report-destination conflict was
  added afterward to the refusal fixture and passed in the affected selection.
  The later equal existing membership/report version-7 case passed in scoped 8/8.
- `mise run //packages/kuru-memory:test -- conditional_state`: terminal exit 0,
  5/5 passed, 11.78 s. Exact complete live/candidate request boundary, escaped
  overflow with no effects, real races/rollback/version invalidation, schema-8
  history and managed stale/lost-reply exact recovery.
- `KURU_BLESS_PROTOCOL_PIN=1 mise run //packages/kuru-memory:test -- protocol_surface`:
  terminal exit 0, 1/1 passed. Generated protocol 1.9 golden reviewed: four
  read-cut calls, two response names and exact candidate/cursor request shapes.
- Runtime owner: `mise run //packages/kuru-runtime:test -- topology_state_tests`:
  terminal exit 0, 7/7 passed, 5.06 s (five real-Dolt, two pure). Retained extras,
  archived/slash/long identities, count/byte fallback, owned report/session rows,
  membership stale refusal, relationship admission and membership-only dream/undo.
  Existing whole-source closing-scope guard separately passed 1/1.
- Runtime existing seven-module OR regression invocation: terminal exit 101,
  98 passed and 2 failed, 77.53 s. One obsolete fixture saved an intended member
  instruction mutation through the now-session-only seam; it now uses exact
  membership publication without changing assertions. One real production
  regression read the model before reconciling a pending durable model choice;
  `set_effort` now reconciles first. Both corrections were independently reviewed.
  Only those failed cases were rerun: terminal exit 0, 2/2 passed, 4.37 s behavior
  (301.58 s task duration included shared Cargo waits and supervisor preparation).
- Memory affected OR regression selection: terminal exit 101, 131 passed,
  6 failed and 2 explicitly ignored measurements, 560.10 s. Five failures were
  exact stale current-schema/count/synthetic-future assertions; historical v8
  assertions remain unchanged. The sixth template-concurrency fixture observed
  two Created outcomes after concurrent owning preparation changed the profile's
  current supervisor receipt while the parent retained its cached old snapshot;
  its new child resolved the new snapshot, making distinct template fingerprints.
  No production/contract/privacy/deadline change was made for that failure.
  The six failed cases plus the changed equal-destination and encoding-parity
  fixtures were rerun once with stable source and no competing prefetch:
  terminal exit 0, 8/8 passed, 40.54 s behavior (213.60 s including preparation).
  The original Created+Reused assertion passed unchanged. This is scoped final
  acceptance of the failures and changed cases, not a repeated full 139-case run.
- Independent memory/runtime source reviews found no blocking defect in the
  accepted scope, including final fixture and owner-local static corrections.
- Final common `mise run lint`: terminal exit 0, 97.12 s. Its all-target,
  all-feature owning Rust checks also compile core's updated test contract.
  `mise run lint:windows`: terminal exit 0, 135.69 s; this is cross-target lint,
  not native Windows behavioral execution. Initial common attempts exposed a
  missing core test StateKeys literal and an unnecessary runtime arity lint
  expectation; their narrow owning corrections were independently reviewed.
- Final `mise run format:check`: terminal exit 0. `mise run docs:check`:
  terminal exit 0, including VitePress build and published content/link/anchor
  verification. `mise run cospec:managed:check`: terminal exit 0, no drift.
  Final `mise run cospec -- validate split-topology-persistence --strict`:
  terminal exit 0, zero errors and warnings. Sandbox cache-access refusals for
  Windows lint and managed check were retried with normal authorized cache
  access; neither altered source or validation policy.

Surface declarations were corrected after implementation to name integration and
agent-behavior effects; this does not retroactively change the original authored
artifacts. Updated strict/apply first exposed two soft blockers. After the required
integration-contract section and bounded existing-fixture offline evaluation row
were authored, strict validation passed and actual apply exited 0; all eight
returned context files were read in full.

Root rechecked origin/main remains e6fa5d864f9c884c394cfb481516c4528e156013,
already this branch's base; no restack is currently needed. Main is rechecked
before push, with any newly merged changes integrated normally. Actual archive
and normal commit follow this completed local record. No standalone core test
suite, full native suite or 90% coverage run executed locally: core's small key
change is compiled by all-target lint and exercised through the runtime codec
fixtures; broader native/coverage acceptance is hosted after delivery.
