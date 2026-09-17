## Why

The published `apps/kuru-docs` site never documented Phase 1's permission engine
and still stated the pre-Phase-1, now-false behavior of `allow_write`/`allow_shell`
as a hard block. Root `docs/` already describes the shipped engine correctly; the
site needs the same substance, in its own voice, plus an explicit upgrade note for
the legacy-boolean behavior change (verified merge-blocker from PR #37 review).

## What Changes

- `apps/kuru-docs/reference/configuration.md`: replace the stale "Tool permissions"
  section with the `[[permissions]]` array (allow/ask/deny; native/mcp/a2a
  selectors; anchored project-relative path patterns for native file tools only),
  the "any matching deny wins, then ask, then allow" precedence rule, once/session/
  always TUI approvals, and an upgrade note covering the `allow_write`/
  `allow_shell` behavior change with the fix (an explicit `deny` rule).
- `apps/kuru-docs/reference/tools.md`: replace the built-in tools permission table
  and the shell-authority paragraph so they reflect the permission engine and the
  ask-not-block fallback, including headless structured permission-required denial.
- `apps/kuru-docs/reference/mcp.md`: note that an MCP tool selector can also be
  targeted by a `[[permissions]]` rule.

## Impact

Readers of the published docs site configuring `[[permissions]]`, `allow_write`,
or `allow_shell`, or relying on the old hard-block behavior for existing configs.
No code, schema, or root-docs changes; root `docs/` was already correct.
