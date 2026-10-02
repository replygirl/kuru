# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Consumers

Not gating; recorded so the dependent change can rely on this one deliberately.

- The open-time gate change (`open-time-report`, PR #133, branch
  `ci/open-time-report`, not merged and not active on main): this change
  unblocks it. Its phase 2 is expected to turn open-time reporting into a gate
  for the installed binary's cold first launch. Before this change, the native
  mise fixture's first launch failed on Windows coverage partitions with
  `memory service readiness deadline exceeded` (4 of 37 runs), so a gate on
  first-launch time would inherit that flake. This change removes the fixture's
  template build and engine extraction from its timed launch, and adds the
  `embedded_runtime first launch (cold cache, …)` line to the
  `test:embedded-runtime` install job log on every OS as the cheap cold-path
  signal that change consumes. This change does not wait for it.
