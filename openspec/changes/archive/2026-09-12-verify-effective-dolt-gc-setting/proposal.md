## Why

The generated Dolt server configuration disables automatic garbage collection,
but the pinned real engine's effective startup value is not asserted.

## What Changes

- Add one assertion to the existing isolated native server-lifecycle fixture
  that queries `@@GLOBAL.dolt_auto_gc_enabled` from the pinned Dolt engine.
- Record the observed effective value as verification evidence without changing
  YAML, garbage-collection behavior, schema, or stored data.

## Impact

Only `packages/kuru-memory/tests/server_lifecycle.rs` and this chore's
artifacts change. The focused native lifecycle test covers the added query.
