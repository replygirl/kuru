## Why

The Release workflow currently makes the GitHub release public before its native Windows installation acceptance runs. A verification failure can therefore leave a publicly downloadable release whose required release workflow did not complete, which violates the requested publication boundary and was observed after the v0.3.0 and v0.3.1 releases.

Publication must be the workflow's last operation and must depend on every required candidate, Windows, and documentation acceptance result. Validation that needs release-shaped bytes must operate on a complete staged candidate without claiming it exercised a public download.

## What Changes

- Assemble and validate one artifact containing the exact five native archives, their checksum sidecars, generated `SHA256SUMS`, and release notes without making a remote release write.
- Run the existing native mise acceptance against the exact staged Windows ZIP and build the release documentation before publication; deploy the built documentation only after both checks succeed.
- Make release publication the sole final job, with no dependent child jobs, and require the staged Windows acceptance and documentation deployment before it can create, complete, reuse, or promote the GitHub release.
- Preserve the existing draft digest validation, immutable recovery behavior, exact selected commit, and manual published-Windows verifier as an optional post-publication diagnostic rather than a workflow gate.
- Update release documentation and repository guidance to describe the new ordering and the distinction between staged loopback acceptance and an actual public download.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `release-automation`: Require complete staged acceptance and documentation deployment before the sole final publication job, and correct the native archive inventory to five.
- `repository-delivery`: Verify the exact staged Windows release candidate through native mise before publication while retaining the separately invokable published-package verifier.
- `native-windows`: Move required release acceptance to the exact staged Windows archive before public promotion without representing loopback delivery as a public download.
- `public-documentation`: Build and deploy release documentation from the selected commit before public release promotion and document recovery at the pre-publication boundary.

## Impact

The Release workflow, delivery-owned release/candidate tooling and tests, native mise acceptance fixture, release documentation, installation guidance, and `AGENTS.md` release-ordering rule are affected. Runtime behavior, archive contents, dependencies, public installation commands, release credentials, and the immutable published-asset contract do not change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
