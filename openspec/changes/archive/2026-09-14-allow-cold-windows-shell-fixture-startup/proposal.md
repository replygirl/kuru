## Why

The isolated Windows environment-projection fixture reached its 30-second ToolHost default before the first authored statement created its stage file. The owned PowerShell process and pipes were still live and cleanup succeeded; this localizes the delay to cold engine or bootstrap startup without establishing which startup operation was responsible.

## What Changes

- Give only this fixture's ToolHost call an explicit 60-second operation allowance.
- Derive its parent child wait and outer watchdog as 70 and 75 seconds, preserving the existing cleanup margins.
- Keep every environment, module, stage, output, and cleanup assertion unchanged.

## Impact

Only `packages/kuru-connectors/src/tools.rs` test code changes. Production shell defaults and maximums, launch and module bootstrap behavior, retries, and runtime policy remain unchanged; native Windows remains required acceptance.
