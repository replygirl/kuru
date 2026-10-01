# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Related shipped work

- `windows-powershell-bootstrap-autoload` (archived 2026-09-25) applied the
  same exact `$PSHOME` imports to the release bootstrap that this entrypoint
  forwards to; this change extends the guard and depends on nothing unshipped.
- `windows-stock-shell-module-bootstrap` (archived 2026-09-14) applied the
  guard to the ToolHost stock shell.

## Related open work

- `windows-probe-trace` (PR #161, open, not on main) instruments the same
  source-entrypoint probe in `apps/kuru-tui/tests/windows_cli.rs`. This change
  leaves the probe body and fixture layout untouched and appends its test at
  the end of the file, so the two merge independently in either order. A
  one-line Utility import in the probe's `finally` is a possible follow-up
  after #161 lands; it is not part of this change.
