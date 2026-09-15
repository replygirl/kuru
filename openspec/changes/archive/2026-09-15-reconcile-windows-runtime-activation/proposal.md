## Why

The staged Windows release acceptance for release run `34925918389` completed the bundled runtime version probe, then failed activation when `MoveFileExW` returned access denied. Fresh checked observations proved that the held source still had its expected identity and the destination was absent, but the current memory filesystem helper reports that proven no-move result as uncertain and provides no bounded activation recovery; the owner of the transient denial remains unknown.

The durable-transition contract already requires identity reconciliation before retrying an uncertain native publication. The implementation needs to distinguish a checked no-move outcome from a move that may have happened so it can retry only the former without weakening identity, staging, locking, or publication guarantees.

The first PR validation at exact head `d8242fe95358db6428a613846f215f62a583124c` then failed all four Windows coverage shards during compilation before runtime behavior. The coverage helper retained Cargo's JSON stream in a runner-local file but did not render the underlying compiler error, so this fix also needs the supported diagnostic-bearing JSON mode without changing the inventory contract.

## What Changes

- Classify both checked publication outcomes: destination has the source identity and source is absent means moved successfully; source retains its expected identity and destination is absent means no move occurred.
- Preserve the original typed publication error while exposing the proven outcome to the activation owner.
- On Windows, retry bundled-runtime activation for at most two seconds at 20-millisecond spacing only after an access-denied error and an explicit proven no-move outcome, while retaining the same source handle, cache lock, and private stage.
- Stop immediately for other errors, occupied or rebound paths, observation errors, and genuinely uncertain outcomes.
- Add native controls for recovery after a held descendant causes the first denial, persistent blocking and cancellation bounds, and refusal to retry unsafe or uncertain states; retain staged release acceptance as the hosted end-to-end proof.
- Preserve Cargo's Windows coverage inventory JSON while rendering native compiler diagnostics to standard error when a checked shard cannot compile.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `native-windows`: Clarify the checked no-move activation outcome and the narrow bounded Windows retry permitted after access denial.

## Impact

The primary implementation surfaces are `packages/kuru-memory/src/files.rs`, `packages/kuru-memory/src/provision.rs`, and their owned native regressions, plus `packages/kuru-delivery/support/windows-coverage.ps1` and a brief correction in `docs/development.md`. No public API, archive format, runtime dependency, release asset, production timeout outside this activation recovery, or non-Windows publication behavior changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
