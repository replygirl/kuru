# Design

## Context

The three checked cold-copy byte loops omit the existing progress seam. The failed native run's owner-running observation cannot establish the version child's state; preparing a copy, creating the child and reaping its owned descendants precede different observable boundaries.

## Decisions

Share one ByteTicks counter over source hashing, accepted copy writes and destination hashing. Its only input is completed byte work; diagnostic phase marks never advance it. Keep checked objects and the stage lease retained through actual process cleanup.

Use the existing exact test-support startup gate and stderr channel for bounded static phase labels, elapsed times and numeric/boolean pipe facts. Forward a read-only snapshot from the retained Windows child handle on refusal, without process control from numeric identities. Preserve normal output, isolated environment and private home.

## Operational surface

The interactive effect is genuine cold-open progress. No bind address, connection limit, secret, binary version, architecture or container/runner topology changes. Diagnostics are inert unless the existing test-support startup gate is selected; native Windows CI supplies Windows process evidence.

## Risks / Trade-offs

The native stall remains unproved. Marks expose the last completed boundary without changing existing probe/readiness timers. Snapshot failure is diagnostic only; cleanup still uses retained authority. Output byte/EOF observations are bounded and exclude bodies.
