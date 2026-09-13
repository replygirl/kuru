## Why

The native stock PowerShell fixture intermittently stalls while resolving an unqualified `Get-FileHash` on hosted Windows, even though the same source has also passed on the same runner image. Qualifying the already-required module avoids the broad installed-module command search while preserving real module auto-loading; the exact cause of the observed stall remains unproven.

## What Changes

- Invoke `Get-FileHash` through its `Microsoft.PowerShell.Utility` module-qualified name in the Windows CLI acceptance fixture.
- Keep the hostile same-name module negative control exact to PowerShell's qualified auto-loading failure.

## Impact

Only `apps/kuru-tui/tests/windows_cli.rs` and its native Windows acceptance evidence change. Production shell behavior, cold cache isolation, deadlines and CI scheduling remain unchanged.
