## Why

Kuru's release workflow verifies candidate Windows packages before publication, but the actual GitHub release and ordinary mise installation path are currently checked only by a manual procedure. The Release run needs a native Windows gate after publication so documentation cannot deploy until the published artifact installs, starts its bundled Dolt engine cold, and reopens durable state successfully.

## What Changes

- Add a package-owned delivery verifier and mise task that exercise an exact published Windows release through the ordinary mise GitHub backend in an isolated environment.
- Verify the published tag, source commit, complete asset inventory, checksum manifest, Windows archive, selected executable, bundled engine and licenses independently of mise's own download checks.
- Run a cold demo conversation, resume, memory status/history and session listing through the installed executable, then emit a bounded JSON evidence receipt after owned cleanup.
- Add one required post-publication Windows job to the existing Release workflow and make documentation wait for it.
- Replace the manual published-Windows procedure with the automated release gate and its bounded recovery guidance.

## Capabilities

### New Capabilities

### Modified Capabilities

- `release-automation`: Require same-run verification of the exact published Windows release before documentation build and deployment.
- `repository-delivery`: Define the package-owned published-release verifier, isolated mise installation, independent artifact checks and bounded evidence receipt.
- `native-windows`: Require a native cold install/resume check against the actual published Windows artifact.
- `public-documentation`: Document the automated published-release gate and same-run recovery path.

## Impact

The delivery tool CLI, a delivery-owned verifier module and tests, the delivery mise task, `.github/workflows/release.yml`, release documentation and curated installation/release documentation are affected. The verifier reuses existing exact pins and archive primitives and adds no dependency from delivery to the application, memory or TUI crates. There is no storage migration, new workflow entrypoint, release credential, publication policy or user-facing Kuru JSON change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
