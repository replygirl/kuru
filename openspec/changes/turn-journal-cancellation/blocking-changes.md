# Dependencies

## Blocked by

None.

## Soft-blocked by

- [ ] `mcp-server-degradation` — its reviewed implementation is present at this branch's exact base `4de6b2a`, providing retained stdio ownership, alias-local no-replay state, and bounded cleanup after MCP caller loss; native Linux/Windows evidence and archive delivery remain pending
- [ ] `owned-unix-shell-lifecycle` — its reviewed `6f83ee4` constituent is present in base `4de6b2a`, providing registered Unix shell ownership and cleanup that survives caller/runtime loss; remaining native evidence and archive delivery remain pending

## Siblings

`async-terminal-events` owns the terminal scheduler and generic abnormal-exit cleanup; this change replaces normal operation cancellation with an explicit token while preserving that scheduler and its generation fence. `bounded-provider-retries` retains its own operation budget and conservative transport replay rules when the containing turn is cancelled. `ordered-dolt-migrations` does not block this change because journal values use existing application state and message tables without a schema change. A full memory export naturally includes these ordinary state and transcript rows and needs no journal-specific table inventory.
