# Proposal

## Why

The cold install's checked source hash, private executable copy and copied-file hash complete real bounded work without advancing the existing owner progress count. Main CI run 37352399670's Windows coverage partition 7 stopped in CheckingRuntimeVersion after 30 seconds without progress; its retained owner observations do not reveal whether the cold probe reached process creation, completed its root process, drained its pipes or entered cleanup.

## What Changes

- Thread the existing per-open byte counter through all three checked cold-copy loops, advancing only for completed 8 MiB units. Preserve source and destination checks, probe isolation and owned cleanup.
- Add bounded cold-probe observations to the existing explicitly gated startup diagnostic channel: checked-copy phases, private-home preparation, native creation return, wait refusal and cleanup entry/exit.
- On refusal record bounded pipe byte/EOF facts and a read-only retained-child snapshot. Keep diagnostic marks separate from progress and never expose pipe contents.
- Add regressions for genuine progress and a blocked copy, plus observed pipe/refusal facts and diagnostic privacy. Document the observations for native diagnosis.

This corrects a demonstrated missing-progress path and supplies missing evidence. It does not establish or claim to repair the unidentified native stall; existing probe and readiness timeout policies remain unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing project-memory-owner bounded-work progress requirement already covers this work.

## Impact

Owning memory provisioning/progress tests, a read-only child diagnostic forwarding seam in memory's engine wrapper, and contributor startup diagnostic documentation. No new dependency, workflow, storage schema, protocol, output payload, process authority or readiness deadline.

## Surfaces

- [x] interactive — cold memory readiness reports genuine progress
- [ ] deploy — no runtime or CI execution topology change
- [ ] integration — no external contract change
- [ ] agent-behavior — no prompt or routing change
