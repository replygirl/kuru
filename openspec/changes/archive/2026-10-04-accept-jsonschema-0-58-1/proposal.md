# Proposal

## Why

Accept Dependabot PR #139 after reviewing its upstream patches and verifying
Kuru's published configuration schema contracts against the locked dependency.

## What Changes

- Preserve the existing `Cargo.toml` exact jsonschema pin update from 0.58.0 to
  0.58.1 with default features disabled.
- Preserve the existing `Cargo.lock` updates for jsonschema 0.58.1 and
  jsonschema-regex, jsonschema-value and referencing 0.58.3, including checksums.

## Impact

The dependency is inherited by kuru-core's configuration schema tests. No
application source, public configuration contract, tooling pin, installation step
or release workflow changes are required. Existing Dependabot attribution remains
in the rebased dependency commit. Local acceptance covers upstream compatibility,
the owning core suite, locked compilation and repository static/docs checks.
Archive and final hosted checks must succeed before merge.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
