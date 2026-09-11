## Why

Coverage compiles and snapshots an ordinary memory supervisor that it explicitly
does not use. Prepare only its verified bundle inputs, then let the single
instrumented build supply its supervisor and exercise cold engine startup.

## What Changes

- `packages/kuru-delivery/mise.toml`: replace coverage's ordinary prefetch
  dependency with memory-owned bundle preparation, retaining upstream ZIP fixtures.
- `docs/development.md` and `AGENTS.md`: clarify ordinary versus instrumented
  preparation without changing package ownership or test guarantees.
- `docs/release.md`: correct stale wording about validation installing hk/setup.

## Impact

The existing coverage command, 90% threshold, native jobs, release dependencies
and secrets remain unchanged. Ordinary package tests retain their prepared
supervisor snapshots. Observe cold instrumented execution locally and on native
CI before recording completion; historical preparation time is not a promised
performance improvement.

## Surfaces

- [ ] interactive — no interactive behavior changes
- [x] deploy — local and hosted coverage preparation topology
- [ ] integration — no external contract changes
- [ ] agent-behavior — no runtime or agent behavior changes
