## Why

The foreign-repository fixture snapshots Git's entire repository immediately after
creating its sentinel commit. Hosted Linux and macOS runs caught an automatic Git
maintenance lock changing after that baseline, so background fixture setup could
look like a repository-isolation failure.

## What Changes

Disable automatic maintenance for the disposable sentinel's setup commands before
creating the baseline. Keep the complete byte snapshot assertion, with no lock
exclusions or sleeps. Use real Git tracing to verify setup does not launch a
maintenance child, then rerun the hook-environment scenarios and hosted gates.

## Capabilities

### Modified Capabilities

None. This corrects test lifecycle isolation without changing the product contract.

## Impact

Only the shared delivery integration-test helper and its verification records.
No production commands, global Git configuration, dependencies or workflows change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration
- [ ] agent-behavior
