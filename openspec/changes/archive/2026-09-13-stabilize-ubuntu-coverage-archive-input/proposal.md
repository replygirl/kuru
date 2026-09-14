## Why

Ubuntu's instrumented test-profile Kuru executable can exceed the existing 128 MiB expanded release-archive bound before the real packaged-runtime fixture runs. Coverage maps do not require Rust test-profile debug information, so that one CI step can retain the full graph, tests, instrumentation, and threshold with a smaller selected executable.

## What Changes

- Run only Ubuntu's existing coverage step with test-profile debug information disabled.
- Log the selected executable size and unchanged expanded-archive limit before the packaged-runtime fixture calls the production archive boundary.
- Document the Ubuntu coverage backtrace tradeoff for contributors.

## Impact

Touches the reusable native-test workflow, its existing embedded-runtime test, and contributor documentation. Production archive limits and behavior remain unchanged; macOS, Windows, local tasks, test inventory, and the per-OS 90% line gate retain their current configuration.
