## Why

The native PowerShell regression limits each captured stream to 64 KiB but waits for both readers to finish before checking the limit. If a child writes beyond one stream's limit and blocks before closing the other stream, capture reports a timeout instead of the output-limit failure. Timeout assertions also discard partial output and launch identity, while failure cleanup is left to drops.

The original Windows timeout in CI run 34630641990 exposed these failure-handling gaps, but its underlying PowerShell stall remains unconfirmed. Repeated original and diagnostic scripts, including runs across 16 fresh Windows runners, passed; this change does not claim to correct that historical stall.

## What Changes

Make the native regression's capture helper reject a stream as soon as its retained bytes exceed 64 KiB. Drain stdout and stderr concurrently within the existing deadline, preserve bounded partial output and the direct/configured launch label, and distinguish the retained root process state from owned job quiescence when capture fails. Await owned child and pipe cleanup before reporting the failure.

Add deterministic native controls for a stalled child and a child that writes beyond a stream limit. Require the oversized writer to produce an output-limit error before the overall timeout and require both failure paths to reap the owned process.

Keep the original PowerShell 5.1 script, actual .NET initialization, 30-second deadline, isolated environment, and single direct/configured comparison. Restore the normal single Windows platform CI job and ordinary artifact name after the temporary diagnostic matrix.

## Capabilities

### Modified Capabilities

None. The existing command execution contract remains correct.

## Impact

The correction belongs to the test capture helper in `packages/kuru-platform/tests/windows_commands.rs` and the native process fixture needed for its failure controls. No public API, production process boundary, dependency, model/provider, or release workflow changes are planned. The temporary diagnostic CI matrix and stress repetitions do not remain in the final change.

## Surfaces

- [ ] interactive — no CLI or TUI changes
- [ ] deploy — ordinary CI topology restored; no final topology change
- [ ] integration — no provider or protocol contract changes
- [ ] agent-behavior — no runtime behavior changes
