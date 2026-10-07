# Proposal

## Why

The native Windows theme contract fixture compares a raw path against the already specified terminal-safe configuration provenance. Windows backslashes are escaped in that display, so the new fixture fails despite the correct source rendering.

## What Changes

- Update only the expected local configuration source in `packages/kuru-core/tests/config_schema.rs` to the established escaped, bounded display contract. Preserve all schema, palette, layer precedence and no-authority assertions.

## Impact

No production, schema, dependency, workflow, deadline or coverage-policy changes. Run the owning theme contract case and affected statics; fresh native Windows CI remains required before merge.
