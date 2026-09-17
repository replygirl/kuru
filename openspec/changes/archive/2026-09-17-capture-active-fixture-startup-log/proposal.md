## Why

The observer fixture bypasses the opt-in startup diagnostic helper and an owned Dolt pre-readiness failure can name the active project log while the helper only scans staging.

## What Changes

- Cover the existing active-project fixture log fallback and map the observer fixture's awaited opening error through the test-only helper.

## Impact

Test-only diagnostic coverage; no production API, retry, deadline, or log-privacy behavior changes.
