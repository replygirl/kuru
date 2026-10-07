# Proposal

## Why

Windows owned command output aborts its diagnostics sampler without awaiting it.
The sampler retains a duplicate query-process handle and can outlive the settled
output call despite the existing ownership comment. An updater fixture's root
retirement reported OS32 after its body passed; its actual locker is unknown.

## What Changes

Abort and await the same optional diagnostics sampler before returning either
the captured success or primary error. Ignore expected diagnostic-only join
cancellation; preserve the output, child/Job cleanup, limits and deadlines.

## Capabilities

### Modified Capabilities

None; existing owned completion contract is correct.

## Impact

Only packages/kuru-delivery/src/command.rs changes. No new API, dependency,
platform policy, retry or sleep. Native Windows command ownership/error fixtures
and previous-release acceptance remain required in fresh CI; host checks cannot
prove a cure for the historical OS32 failure.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
