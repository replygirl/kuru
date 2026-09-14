## Why

The isolated Windows ToolHost fixture repeatedly times out at its first stock
PowerShell cmdlet, while later controls complete and do not identify the blocked
operation. One bounded native and managed stack-text capture can localize the
existing failure without changing shell behavior, deadlines, or pass criteria.

## What Changes

- Add a test-only, explicitly selected CDB/SOS stack capture to the existing
  isolated Windows shell fixture and prove it against an owned sleeping process.
- Replace the prior inconclusive explicit-import failure control while preserving
  the authoritative shell source and outcome unchanged.
- Prepare the exact Microsoft Debugging Tools input only for the Windows
  connectors/core/platform coverage shard and validate its workflow boundary.
- Align the fixed CDB option order with the documented command-file-before-PID
  form and retain at most 4 KiB of terminal-safe, redacted failure output when
  capture validation rejects, so the exact isolated fake fixture can expose the
  attach or command failure without inspecting stores or arbitrary processes.
  Exclude the target command line and debugger prompt-command echoes, and state
  explicitly when projection leaves no output.
- Keep the original ToolHost result authoritative and require bounded debugger
  and target cleanup regardless of capture outcome.

## Impact

Touches the connector shell fixture, package-owned delivery setup task and
support script, native-test workflow validation, and the Windows connector shard.
That shard gains one debugger setup and bounded diagnostic interval; production
code paths, normal timeouts, other shards, coverage gates, and release behavior
remain unchanged.
