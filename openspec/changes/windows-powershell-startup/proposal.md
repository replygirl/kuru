## Why

The Windows platform job in CI run 34630641990 timed out while draining the stock PowerShell regression's output. The same regression passed in the full Windows job at the identical released commit, and the failing assertion discarded the launch label, partial output and child state, so the available log does not identify the cause.

## What Changes

Reproduce and identify the failing stage of the native PowerShell launch, then correct the established cause within the process boundary or its fixture. Preserve the regression's real PowerShell 5.1 and .NET initialization, its isolated environment, its existing deadlines, and its direct/configured launch comparison. Capture useful failure evidence and await owned child and pipe cleanup when the regression fails.

Release scheduling, publication, provider authentication, runtime evaluations and unrelated Windows behavior remain outside this fix.

## Capabilities

### Modified Capabilities

None. The existing native command execution contract remains correct.

## Impact

`packages/kuru-platform` owns the command boundary and native regression. A correction will be limited to that boundary and the fixture or package-owned verification needed to establish the cause; no dependencies or public API changes are planned.

During diagnosis only, the existing Windows platform job runs on 16 fresh runners, each exercising the original single direct/configured comparison. This distinguishes fresh Windows instances from repeated processes sharing one instance. Restore the normal single platform job and artifact name before merge; no release workflow changes are involved.

## Surfaces

- [ ] interactive — no CLI or TUI changes
- [x] deploy — temporary native CI reproduction matrix, removed before merge
- [ ] integration — no provider or protocol changes
- [ ] agent-behavior — no runtime behavior changes
