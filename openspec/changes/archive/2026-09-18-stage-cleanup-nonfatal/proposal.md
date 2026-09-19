## Why

Post-publication cleanup of the private install stage is fatal: `StagedActivation::finish_published` (`packages/kuru-memory/src/provision.rs:588`) `?`-propagates `PrivateTemp::close` failures, so `MemoryStore::open` (`packages/kuru-memory/src/store.rs:728`) fails after the engine was already published and verified. On the Windows memory-runtime shard this fails cold installs intermittently: an external holder of the just-executed private probe `dolt.exe` outlives the bounded two-second cleanup window, producing "Dolt engine publication succeeded, but private stage cleanup failed … exhausted its bounded recovery after 88 reconcile attempts over 2.003 s … Uncertain removal … The directory is not empty (os error 145)".

The roadmap author has ruled that this fatal contract over-reads the roadmap: publication outcomes must stay exact, but reclaiming storage belongs to a later collection pass, and only integrity failures (corrupt bytes, identity mismatch, privacy rejection) are fatal at startup. AGENTS.md says the same: do not force the deletion, reconcile later.

## What Changes

Post-publication private-stage removal becomes report-and-continue. When `close` exhausts its existing bounded window, the stage is kept exactly as an abandoned activation already keeps it, a small private JSON receipt is written beside it, and the open proceeds with the published, verified engine. A receipted stage is never verified, opened for execution, or mistaken for a live engine. A later cold provision — and a warm open that finds receipts — sweeps receipted stages under the existing installation lock using the existing checked identity-verified removal, leaving anything still uncertain for the next pass. Leftovers are capped by count for reporting only. One stderr line at startup and one diagnostics-ring record surface the report.

Publication exactness is unchanged: rejected stays rejected, uncertain-retained stays retained, the two-second cleanup bound is unchanged, and no stage whose removal is still uncertain is deleted.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/files.rs` (stage close outcome and typed exhaustion), `packages/kuru-memory/src/provision.rs` (activation completion, receipts, sweep, non-blocking lock attempt), `packages/kuru-memory/src/progress.rs` (one open stage), `apps/kuru-tui/src/cli.rs` (one stderr notice), `docs/memory.md` and `apps/kuru-docs/concepts/memory.md`. No public API, platform primitive, configuration key, publication policy, process lifecycle, or timeout change.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
