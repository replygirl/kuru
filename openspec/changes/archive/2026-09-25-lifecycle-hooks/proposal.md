## Why

Kuru has no reviewed extension point at turn, tool, or speaker-selection boundaries, so a project cannot apply deterministic local policy without replacing runtime code. Lifecycle hooks must join the existing workspace authority, permission, mode-policy, and parallel-tool contracts so external commands can influence only the event fields canon permits.

The Phase 2 roadmap and its author response define the safety boundary: pre-event hooks may allow, deny, or rewrite before final validation; post-event hooks may annotate settled work without changing it; and speaker hooks may stop but never replace the selected peer. Implementing those rules as one bounded capability avoids a generic plugin runtime or a misleading process-sandbox claim.

## What Changes

- Add strict, provenance-aware configuration for ordered `pre_turn`, `post_turn`, `pre_tool`, `post_tool`, and `speaker_selected` command hooks.
- Execute hooks as owned, bounded connector commands with structured versioned input/output, finite environment, timeout, output, count, and recursion limits, and complete cancellation cleanup.
- Apply deterministic pre-hook rewrite precedence, then rerun ordinary shape, budget, root, permission, and mode-policy validation against the final value before dispatch.
- Preserve every settled tool effect, answer, durable record, and speaker choice; record bounded post-event annotations separately, admit visible annotations through ordinary context limits, and never trigger an implicit turn.
- Integrate tool hooks with the archived parallel-call ordering contract so admission, rewrite, denial, execution, settlement, annotation, and provider continuation remain deterministic by original call identity.
- Document hook process authority, workspace-trust review, failure behavior, and the absence of a generic plugin runtime or OS sandbox.

## Capabilities

### New Capabilities

- `lifecycle-hooks`: Ordered, bounded lifecycle-hook configuration, execution, rewrite, stop, annotation, failure, and cleanup semantics.

### Modified Capabilities

- `configuration-schema`: Publish the strict hook configuration shape, bounds, and parser/schema parity.
- `workspace-trust`: Bind effective repository hook commands and settings into exact-root reviewed authority before activation.
- `provider-tools`: Revalidate rewritten tool operations through the existing evaluator and preserve settled results under post hooks.
- `parallel-tool-execution`: Place pre/post tool hooks in deterministic per-call admission and ordered continuation semantics.
- `mode-policy-dispatch`: Allow speaker observation or stop only after normal policy selection, without speaker substitution or new authority.
- `typed-runtime-events`: Project bounded hook outcomes and annotations without exposing raw hook input, output, or command details.
- `public-documentation`: Explain configuration, authority, ordering, annotations, failures, cancellation, and process limits.

## Impact

The change affects hook configuration and authority types in `packages/kuru-core`, owned command execution in `packages/kuru-connectors`, turn/tool/speaker orchestration and typed events in `packages/kuru-runtime`, user-visible projection and PTY acceptance in `apps/kuru-tui`, the published configuration schema, and configuration/tool/protocol documentation. It builds on archived P01 workspace authority and P06 parallel-tool ordering and adds no storage migration, provider dependency, generic plugin interface, or new public network protocol.

## Surfaces

- [x] interactive — hook denials, failures, and annotations affect user-visible turn/tool output
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — hooks can rewrite admitted inputs or annotate actor context within fixed authority
