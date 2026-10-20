# Proposal

## Why

A memory client waits for the owner it spawned with a `memory.startup_timeout_secs` window (30 s by default) that restarts whenever the owner's open activity record changes, and `progress.rs` states that every milestone stamped inside an open is also a progress point. When a new project is created from the store template, the owner reports nothing from its `OpeningDatabase` stage report until the stage engine's `supervisor-ready`: `supervisor-spawned`, `channel-accepted`, `template-copied` and `activated` are bare timeline stamps, the completed template copy and the stage identity write advance nothing, and the creation worker's stage start reports through a silent reporter. On a degraded Windows runner (CI run 39455388792, job 118741241523, `native_roaming_and_local_defaults_and_explicit_precedence_survive_real_reopen`) that silent stretch exceeded the window, so the client abandoned a still-working owner with `memory service readiness deadline exceeded` and fixture cleanup then timed out on an owner still opening. The timeline also had no stamp between `create-start` and `supervisor-spawned`, so the failure could not show which step was slow.

## What Changes

- `supervisor-spawned`, `channel-accepted` (Windows), `template-copied` and `activated` become progress milestones: each keeps its timeline stamp and advances the open's own count.
- The creation worker advances the count when the template copy completes and when the stage identity is written, and its stage engine start reports through a counting reporter that publishes no stage, so its stage reports and milestones advance the count while the client's stage sequence is unchanged.
- Four new open-timeline milestones localize the creation span: `legacy-prepared`, `recovery-checked`, `copy-end` and `spawn-call` (before every engine supervisor spawn).
- No timeout value, startup deadline, stage, record format or client wait rule changes.

## Capabilities

### New Capabilities

### Modified Capabilities

- `project-memory-owner`: the tagged open activity record's count is required to advance at the open's timeline milestones and across a template creation, not only restricted in when it may advance.

## Impact

`packages/kuru-memory/src/{progress.rs,open_timeline.rs,server.rs,store.rs,store/creation_worker.rs,store/creation_template.rs}`, their tests (`service.rs`, `service/open_timeline_tests.rs`, `open_timeline.rs`), and `docs/development.md` (progress points, canonical timeline events, reading `last`). No public API, configuration, dependency, TUI or protocol change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
