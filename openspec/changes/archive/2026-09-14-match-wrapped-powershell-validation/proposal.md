## Why

The native Windows regression reached the intended PowerShell wrapper validation, but PowerShell wrapped the long error sentence across two lines. Matching the full sentence made the test fail despite proving the required boundary.

## What Changes

- Match the stable unique `Published Windows verification requires` prefix in `packages/kuru-delivery/tests/powershell_diagnostics.rs`.
- Preserve the actual mise invocation, forced cmd boundary, removed inputs, nonzero status, and rejection of the prior cmd parse error.

## Impact

Only one test assertion changes. Production task, wrapper, verifier behavior, and CI topology remain unchanged.
