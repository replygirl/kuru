# Dependencies

## Blocked by

None.

## Soft-blocked by

- [ ] `owned-unix-shell-lifecycle` — shares the ToolHost shell-result boundary and native shell fixtures
- [ ] `shell-environment-minimization` — shares connector tool source, fixtures and curated tool documentation
- [ ] `bounded-provider-retries` — shares connector static/coverage scheduling and protocol documentation

## Later prerequisite

Actual MCP stdio stderr retention is outside this change. Applying this scanner
to a retained stderr excerpt requires a separately applied owned RPC/drain change;
this change proves the streaming scanner synthetically without capturing stderr.
