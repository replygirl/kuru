## Why

The hk pre-push hook runs the full instrumented `mise run coverage` suite on
every push, which dominates push latency while CI already enforces the
behavioral suite and the 90% line-coverage gate.

## What Changes

- `hk.pkl`: remove the `coverage` step from the `pre-push` hook. Pre-push keeps
  the static `format`, `lint`, `typecheck`, `tooling`, `cospec`,
  `cospec-managed` and `docs` steps. `pre-commit`, `commit-msg` and the hk
  version pin are unchanged.
- Ordinary `mise run test` is deliberately not added to pre-push: it compiles
  the test profile, prepares the supervisor snapshot and extracts the bundled
  engine, returning most of the latency. Behavioral tests and coverage are
  enforced in CI, not in hooks.
- `AGENTS.md`, `docs/development.md`, `README.md`: describe pre-push as static
  checks and coverage with its 90% gate as enforced in CI, not in hooks; label the documented
  command chain that includes `coverage` as the full local gate.

## Impact

Local Git pre-push hook only. CI workflows, jobs, required checks, secrets,
mise tasks and the 90% workspace line-coverage threshold are unchanged. No
application source or durable spec changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
