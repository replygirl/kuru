## Why

The first main CI run failed on Linux while the terminal smoke fixture waited
for Kuru to exit. Commands advance after fixed 150 ms delays even when the UI
is busy, and shutdown stops draining PTY output before waiting for the process.
These assumptions allow lost submissions and terminal backpressure under load.

## What Changes

Synchronize terminal fixtures on observable screen and process state with bounded
deadlines. Keep draining output until exit, preserve terminal restoration checks,
and exercise delayed scheduling so fast local runs cannot conceal the race.

## Capabilities

### Modified Capabilities

None. Existing terminal and repository-delivery requirements are correct.

## Impact

PTY integration fixtures, their Rust launchers and verification documentation.
No application UX, provider contract, dependencies or coverage thresholds change.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
