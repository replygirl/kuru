# Dependencies

## Blocked by

None.

## Soft-blocked by

- [x] `mcp-server-degradation` — provides retained stdio ownership, alias-local no-replay state, and bounded cleanup after MCP caller loss. *(archived 2026-09-13)*
- [x] `owned-unix-shell-lifecycle` — provides registered Unix shell ownership
  and cleanup that survives caller/runtime loss. *(archived 2026-09-13)*

## Siblings

`async-terminal-events` owns the terminal scheduler and generic abnormal-exit cleanup; this change replaces normal operation cancellation with an explicit token while preserving that scheduler and its generation fence. `bounded-provider-retries` retains its own operation budget and conservative transport replay rules when the containing turn is cancelled. `ordered-dolt-migrations` does not block this change because journal values use existing application state and message tables without a schema change. A full memory export naturally includes these ordinary state and transcript rows and needs no journal-specific table inventory.
