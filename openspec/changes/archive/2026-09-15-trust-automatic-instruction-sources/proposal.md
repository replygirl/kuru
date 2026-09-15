## Why

Kuru discovers every ancestor `AGENTS.md` and injects its text into model prompts, but those files are currently read after workspace approval and do not appear in the authority manifest. A repository can therefore change prompt authority without invalidating approval, and a pathname replacement between preflight and runtime construction can cause the model to receive bytes the user never reviewed.

## What Changes

- Capture every automatically discovered `AGENTS.md`, including the project root, while building the immutable configuration snapshot.
- Add one ordered automatic-instruction-source claim whose digest binds source identities and exact bytes, and make every command that can construct the runtime review that claim before provider or instruction activation.
- Inject only the snapshot's approved instruction bytes; do not reopen instruction paths after preflight.
- Show bounded source labels and a fixed claim description without exposing instruction contents.
- Document how automatic project instructions participate in workspace approval and invalidation.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `workspace-trust`: Extend the immutable authority snapshot, exact-root approval, command preflight and redacted review flow to automatically discovered instruction sources.
- `chat-harness`: Require runtime prompts to consume the ordered instruction bytes captured in the approved snapshot.

## Impact

`packages/kuru-core/src/config.rs` gains the captured instruction-source representation and claim. `apps/kuru-tui/src/cli.rs`, `apps/kuru-tui/src/trust.rs` and trust integration tests gain command-matrix and review coverage. `packages/kuru-runtime/src/engine.rs` consumes the snapshot bytes. `docs/configuration.md` documents the boundary. No configuration syntax, provider route, global instruction source or persistent record schema is added.

## Surfaces

- [x] interactive — workspace review and refusal output includes the automatic-instruction claim
- [ ] deploy — no deployment or CI topology changes
- [ ] integration — no third-party contract changes
- [x] agent-behavior — approved project instructions are injected into model prompts
