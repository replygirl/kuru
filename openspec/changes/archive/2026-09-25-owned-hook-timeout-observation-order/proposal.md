## Why

The slow Windows lifecycle-hook fixture asserts its start marker and held file lock before awaiting the owned hook future. If either early assertion fails, its spawned task can remain detached while the test exits and removes its private directory, weakening failure cleanup evidence.

## What Changes

- In `packages/kuru-connectors/src/hooks.rs`, poll the marker and lock concurrently with the actual slow hook future, then assert only after that future completes.
- Keep the existing ten-second timeout, eight-second observation bound, started/held-lock/no-later-marker checks, and product behavior.

## Impact

Only the existing Windows test fixture changes; its success path and native CI duration are materially unchanged.
