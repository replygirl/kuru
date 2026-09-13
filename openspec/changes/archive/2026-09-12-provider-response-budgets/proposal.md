## Why

API-key Responses completions inherit the generic 60-second HTTP-client deadline,
while direct ChatGPT subscription completions receive the intended 600-second
operation budget. The model catalogs already retain their 60-second bound, but
the explicit API-key completion route ends long-running valid turns early.

The SSE decoder also counts every wire byte against the 2 MiB retained response
budget. Repeated delta, comment, and framing traffic can therefore reject a
small completed response, while parser buffers and final-envelope reconciliation
need their own finite limits.

## What Changes

- Apply the same 600-second total budget to API-key Responses completions without
  changing the generic HTTP client used by catalog, MCP, or authentication work.
- Separate retained completed-response accounting from bounded SSE wire, line,
  and event-payload limits; reject oversized retained final responses and
  malformed or excessive streams without exposing remote payloads.
- Add local fixture regressions for discarded SSE traffic, retained output,
  finite stream guards, and API-key completion timeout selection.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing provider-tools transport contract already requires direct,
bounded provider execution; these repairs correct its private enforcement.

## Impact

`packages/kuru-connectors/src/providers.rs`, its private SSE decoder, focused
provider/SSE tests, and `docs/protocols.md` change. Public `Completion`,
`TurnOutput`, JSON shapes, provider routing, authentication routes, retries,
configuration, and dependencies remain unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
