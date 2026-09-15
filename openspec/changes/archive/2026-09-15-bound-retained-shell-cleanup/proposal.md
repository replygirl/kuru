## Why

Each accepted Unix built-in shell currently starts its own retained worker without a process-wide admission limit. When cleanup cannot be confirmed, that worker correctly keeps the owned process group, but repeated calls and separate registries can therefore grow retained threads and process ownership without a bound. The indefinite retry path also restarts a five-second, 10 ms cleanup loop, making its actual platform observations frequent even after the caller has received its bounded unconfirmed result.

## What Changes

- Reserve a fixed process-wide retained-shell admission slot before creating a worker or child, and hold it until prelaunch failure or confirmed group cleanup.
- Reject capacity exhaustion immediately with a fixed Kuru-authored error; do not queue callers or create a worker for rejected work.
- Change indefinite unconfirmed cleanup to one phase-safe cleanup observation per capped exponential retry interval while preserving ownership indefinitely and signal-before-reap ordering.
- Cover cross-registry capacity, dropped caller/registry retention, exactly-once capacity release, failure release, and bounded retained observation frequency with deterministic Unix shell tests.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-tools`: Unix built-in shell retained ownership needs a process-wide resource admission bound and a bounded indefinite cleanup observation rate.

## Impact

- `packages/kuru-connectors/src/unix_shell.rs`: retained-shell admission, cleanup retry scheduling, and owned tests.
- `openspec/specs/provider-tools/spec.md`: corrected bounded-shell ownership contract.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
