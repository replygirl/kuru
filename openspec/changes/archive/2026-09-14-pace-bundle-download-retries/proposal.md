## Why

Two consecutive Intel native-build jobs ended in HTTP 504 after roughly 13
seconds, and the latest diagnostic proves the final response was attempt 3/3
without `Retry-After`. The current 250 ms and one-second pauses leave most of
the unchanged 120-second acquisition budget unused during a brief upstream
outage.

## What Changes

- Pace the existing second and third identical GETs with five-second and
  fifteen-second pauses inside the same 120-second total download deadline.
- Add a private delay-injection seam so bounded real-HTTP recovery tests remain
  fast without substituting test behavior globally, plus one focused check of
  the real five-second production pause.
- Replace the retry timing in `docs/development.md`; retain the three-GET cap,
  180-second lock bound, `Retry-After`, status, transport, integrity, staging,
  and publication rules unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `packages/kuru-delivery/src/bundle.rs`, its focused
`bundle/recovery_tests.rs`, and the existing retry paragraph in
`docs/development.md` change. No dependency, workflow, runtime, manifest,
endpoint, cache, or retry classification changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
