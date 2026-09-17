## Why

An intermittent native-CI Dolt startup failure occurs while a temporary test fixture opens. The fixture directory is removed before the caller can inspect the private, already-redacted server log, leaving only an exit status and no cause.

## What Changes

- Capture a bounded server-log tail in the shared test fixture open path when Dolt startup fails, preserving the original error and leaving ordinary opens unchanged.
- Replace the one-test migration diagnostic with focused opt-in, bounds, and affected live-fixture checks.

## Impact

Only `kuru-memory` test-support code and tests change. The diagnostic performs a bounded read on fixture startup failure; successful opens and production behavior have no added work.
