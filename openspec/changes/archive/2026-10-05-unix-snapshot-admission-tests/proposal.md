# Proposal

## Why

Ubuntu instrumented CI at `77e49b87` failed the unreaped-child snapshot fixture
because it unwrapped legitimate `WouldBlock` admission from a concurrent spawn.
macOS instrumented CI observed the same error in the stalled-helper fixture.
The absolute membership API deliberately returns that error before spawning;
these independent fixtures assume admission without testing concurrent snapshots.

## What Changes

- Serialize this integration binary's fixtures with one private test mutex,
  acquired before setup and deadline creation. Recover a poisoned guard so a
  failed fixture does not prevent the others from reporting their own results.
- Preserve every original API call, native live-tree/zombie/selector/error/reap
  assertion, owned-child cleanup and deadline, including the stalled helper's
  100 ms deadline and required PID marker plus owned-row absence.
- Leave the library's deliberate bounded-admission contention controls intact.
  Each fixture's synchronous snapshots finish helper/readers before returning;
  the selector fixture's re-executed test owns a separate process-local lock.

## Impact

One Unix integration-test file and this typed test record only. No production,
helper API, dependencies, workflows, coverage threshold, budget or performance
change. The historical lock holder is unknown. The actual CI failures provide
negative evidence; no claim of reproducing their exact interleaving locally.
A temporary admission retry alone still timed out without a helper PID marker
in the local target run and is removed, along with its adapter-only controls.
Run the owning platform snapshot target and relevant scoped static checks;
fresh native PR CI remains required before merge.
