## Why

Final-head CI exposed two rare native setup failures whose bounded output did
not identify the decisive branch: an Intel bundle preparation ended with a bare
HTTP 504, and an ARM missing-archive bootstrap rejection timed out despite the
same existing instrumented binary passing 25/25 focused repetitions. Their root
causes remain unproven, so this change adds failure-only context without changing
retry, bootstrap, assertion, or timeout behavior.

## What Changes

- `packages/kuru-delivery/src/bundle.rs` adds terminal HTTP status context with
  the target, attempt number out of three, status, and a `Retry-After` presence
  boolean. It adds no URL or header value; reqwest's existing public manifest
  URL/error chain remains available.
- `packages/kuru-delivery/src/bundle/recovery_tests.rs` verifies that bounded
  context for terminal retry-advised and exhausted-status responses.
- `packages/kuru-delivery/tests/bootstrap_install.rs` traces only the final
  missing-archive fixture child with isolated Bash 3.2-compatible xtrace state;
  its existing bounded stderr is still shown only on failure, and the original
  rejection assertion and 30-second deadline remain authoritative.

## Impact

Only delivery automation diagnostics and their focused fixtures change. Native
jobs, required checks, secrets, network policy, download retries, subprocess
ownership, bootstrap behavior, and test deadlines are unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
