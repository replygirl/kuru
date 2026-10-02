# Proposal

## Why

The `memory service readiness deadline exceeded` family (A1) and its supervisor-site variant (`memory supervisor readiness deadline exceeded`) are the remaining readiness failures on main. Every A1 row with a client phase split spends almost the whole 30 s budget in readiness with the child running and no endpoint, but nothing the starter can read at its deadline says which owner phase consumed that budget: the owner open timeline added by `owner-open-timeline` is written only after close, and the starter has already bailed by then. Fixing the family at cause needs the next occurrence to name the owner phase, so this change is the diagnostic that precedes the fix; it changes no wait, bound, retry or budget.

## What Changes

- A gated owner (`KURU_OPEN_TIMELINE` exactly `1`) started with a starter token also streams each open stamp, as one unsynced line, to an owner-private, create-only file `open-stream-<activity tag>` beside its endpoint, and removes that file under its owner lock after endpoint publication and before serving, or on a failed open. A name already taken is never appended to or removed.
- Eleven new open events (`startup-lock`, `extract-start`, `extract-end`, `create-start`, `template-copied`, `cold-created`, `activated`, `migrate-start`, `migrate-end`, `supervisor-spawned`, `channel-accepted`) separate the lock, extraction, creation, migration and per-start supervisor phases.
- A starter whose own environment has the variable exactly `1` reads only its own owner's stream file, once, at its readiness deadline, and adds an `owner timeline: …` clause (or `absent`, `empty`, `stale`, `unreadable`) before the unchanged `client phases:` split. An ungated starter reads nothing and its error text is unchanged.
- The supervisor readiness deadline names the part that had not completed (private channel accept, startup request write, Ready frame) under the unchanged outer cause. On Windows, an inner accept `TimedOut` at or after the shared deadline is reported in that same shape.
- A Windows starter forwards the gate to its owner only when it is exactly `1`, merging test-only owner environment layers without duplicate keys.
- The coverage shard task sets the gate; env-clearing kuru-tui fixtures forward it beside `LLVM_PROFILE_FILE`. Tests that assert ungated behaviour pin `KURU_OPEN_TIMELINE=0`.
- A test-support FIFO hold (`KURU_TEST_MEMORY_TIMELINE_HOLD_DIR`, Unix) lets an end-to-end test hold a real owner at a named event.

## Capabilities

### New Capabilities

### Modified Capabilities
- `project-memory-owner`: the "Owner open timeline" requirement gains the gated stream file, its removal before serving, the gated starter's single deadline read, Windows forwarding of an exact gate, and the bounded per-stamp cost in place of "MUST NOT delay the open".

## Impact

- `packages/kuru-memory`: `src/open_timeline.rs` (stream, gate seam, hold hook), `src/service.rs` (deadline clause, Windows owner environment builder, stream lifecycle in `open_hooked`), `src/server.rs` (readiness part and Windows accept classifier), one-line stamps in `src/store.rs` and `src/provision.rs`; tests in `src/service.rs`, `src/server.rs`, `src/open_timeline.rs`, `src/service/open_timeline_tests.rs`, `src/test_support/usage_scan/tests.rs`; in-code text in `src/test_support/usage_scan.rs` and `src/main.rs`.
- `packages/kuru-delivery/mise.toml`: `coverage:shard` task environment gains `KURU_OPEN_TIMELINE = "1"`.
- `apps/kuru-tui/tests`: env-clearing fixtures forward the gate.
- Docs: `docs/development.md` only; the variable stays developer-only.
- Not breaking. No new dependency, protocol field, migration or configuration. Ungated product behaviour and error text are unchanged. Deliberately unchanged: `startup_timeout_secs` value and meaning, `READINESS_POLL_INTERVAL`, `SUPERVISOR_TRANSPORT_ALLOWANCE`, the single startup deadline instant and both Windows accept timers, the bail strings' leading text, the `client phases:` format, owner/startup locks, reap order and the uncertain-write fence.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
