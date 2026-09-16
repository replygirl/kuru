## Why

Two malformed-stdio tests count a fixture's `.done` marker even though the fixture publishes that marker only after flushing its malformed response. Kuru correctly terminates and reaps the peer as soon as it rejects that response, so scheduler timing determines whether the peer writes `.done`; release verification observed zero of one markers and main CI observed three of four despite the expected requests, degradation, output, redaction, and cleanup behavior.

The tests need to observe the protocol boundary Kuru promises: each expected process received exactly one initialize request, reported the malformed response, and was cleaned up. They must make the old completion-marker assertion fail deterministically without timing padding or weaker process counts.

## What Changes

- Add a trailing read to the two malformed fixture plans so the peer cannot naturally complete after emitting invalid JSON.
- Replace malformed-peer `.done` counts with exact recorded initialize-request transcripts: one connector process and four CLI processes.
- Preserve the existing healthy-alias ordering, degradation, redaction, stdout, event, and confirmed-cleanup assertions.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `packages/kuru-connectors/src/mcp.rs` and `apps/kuru-tui/tests/trust.rs` test code changes. Production behavior, shared fixtures, public APIs, dependencies, workflows, and user documentation are unaffected.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
