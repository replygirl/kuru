## Why

The Windows recovery fixture retained directory handles used only for identity and read provenance, so its intentional leaf blocker could be released while ancestor handles still prevented private-stage disappearance.

## What Changes

- Update `packages/kuru-memory/src/provision/native_tests.rs` to release those fixture-only anchors before activation while retaining the real leaf blocker and all existing publication assertions.

## Impact

Test-only Windows native coverage; no production behavior or CI duration changes.
