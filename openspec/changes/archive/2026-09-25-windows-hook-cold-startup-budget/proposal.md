## Why

Native Windows coverage reached the first successful stock PowerShell lifecycle-hook command but hit its test-only five-second per-command deadline before receiving a reply. The log proves the timeout, while the exact reason the command needed more than five seconds remains unobserved; existing Windows test support allows for slower first PowerShell startup.

## What Changes

- Give the three successful PowerShell commands in `packages/kuru-connectors/src/hooks.rs`'s Windows hook fixture the existing bounded cold-start allowance and a matching aggregate test budget.
- Keep the separate deliberate slow-hook command's ten-second timeout, started marker, owned cleanup, and no-later-hook assertions unchanged.

## Impact

Only the Windows test fixture changes. Product hook defaults and timeout enforcement stay unchanged; the successful fixture may wait longer on a failing native runner.
