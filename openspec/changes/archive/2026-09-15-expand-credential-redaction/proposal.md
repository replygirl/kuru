## Why

Tool-result projection recognizes several credential forms, but it leaves bare
JWTs, Slack `xox*` tokens, GitLab `glpat-` tokens, URL userinfo, and plain
`token=` or `secret=` assignments visible. The current tests deliberately
preserve a JWT-shaped value as an unrecognized control, so the existing
boundary cannot meet the Phase 0 credential coverage requirement.

## What Changes

- Extend the finite streaming recognizable-secret detector with bounded
  recognizers for Slack and GitLab tokens, JWT-shaped three-segment values,
  URL-embedded userinfo, and bare `token` and `secret` assignments.
- Preserve ordinary text through explicit boundary controls and make whole and
  chunked projection agree for every added detector.
- Expose the connector-owned text and JSON projection seam for the later
  runtime event-redaction change without changing existing `ToolHost` callers.
- Document the additional recognized forms and their false-positive limits.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities
- `provider-tools`: recognizable-secret tool-result projection needs the added
  finite credential forms and a reusable connector projection seam.
- `public-documentation`: the tool projection reference needs the current
  detector inventory and its limits.

## Impact

- `packages/kuru-connectors/src/redaction.rs` and its unit tests
- `packages/kuru-connectors/src/lib.rs` projection exports
- `docs/protocols.md` tool-result projection reference
- Existing connector callers retain their public `Result<String>` behavior;
  no provider credentials, tool arguments, file writes, or stored history are
  changed.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
