## Why

The re-review of PR #89 found that the durable pre-turn rewrite provenance record, which the archived `lifecycle-hook-authority-cleanup` change added to each part's private history, was also sent to the model. The runtime passed it as a current input, and the Responses projection turned the unknown `kuru-hook` role into a user message. Every rewritten turn therefore told the model that a hook had rewritten the request, along with internal session, operation, invocation and turn identifiers. The same record would have reached later turns and context compaction through private history. The documented contract says the model receives the rewritten request, and the record exists only as private provenance.

Three smaller defects were in the same area:
- **Cleanup bound.** After a hook's root exited, reaping and the output drain each received their own five-second cleanup bound, and the drain ignored caller loss. A cancellation landing after root exit could therefore keep the worker busy past the host's quiescence bound (`QUIESCE`), which then falsely reported unconfirmed cleanup.
- **Dream settlement.** Dreams refused a call outside the offered dream tool through a separate path with its own message. Deliberation and speaking settle the same rule through the shared pre-tool admission.
- **Strip flags.** The earlier split-out revert (6599e712) removed the macOS packaging strip flags (`-u -r`). The instrumented macOS copy needs them to stay under its 128 MiB cap, and they are restored here.

## What Changes

- **Provenance is private.** Private history keeps the `kuru-hook` pre-turn provenance record, which still immediately precedes the rewritten input. Every provider projection omits it: ordinary requests (current input and retained history) and context compaction. The rewritten text stays ordinary context. Post-hook annotations share the role but are not affected.
- **One post-exit bound.** Root reaping and the output drain after root exit share one cleanup deadline, and the drain stops at the next poll after caller loss.
- **Shared dream settlement.** A dream call outside the offered dream tool goes through the shared pre-tool admission and settles as "tool is not offered in this phase" before any hook runs. The authored proposal cap is still checked first.
- **Strip flags restored.** The macOS packaging-input strip flags `-u -r` are back in a separate commit.
- **Docs.** The durable private record and the Windows guidance on stock PowerShell cold start are documented. Hooks deliberately do not receive the ToolHost `$PSHOME` bootstrap, which AGENTS.md scopes to the built-in shell, so a stock PowerShell hook should set `timeout_ms` to cover a cold start.
- **Accepted, unchanged:** a deliberation `a2a_send` is never offered in that phase, so settling it as an unoffered-tool `Error` without approval is intended behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

- `lifecycle-hooks`: the pre-turn provenance record is excluded from provider projections, and post-exit cleanup (reaping plus output drain) shares one bound and observes caller loss.

## Impact

- `packages/kuru-runtime/src/engine.rs`: record predicate and role constant.
- `packages/kuru-runtime/src/actor.rs`: ordinary request and compaction projections.
- `packages/kuru-runtime/src/dream.rs`: dream admission.
- Tests: `hook_platform_tests.rs`, `hook_tests.rs` and `permission_tests.rs`.
- `packages/kuru-connectors/src/hooks.rs`: post-exit tail and a new test.
- `apps/kuru-tui/tests/embedded_runtime.rs`: strip flags, in a separate commit.
- Docs: `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md`.
- No dependency changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
