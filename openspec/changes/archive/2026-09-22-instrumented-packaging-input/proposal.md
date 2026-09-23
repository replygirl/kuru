## Why

Workspace coverage builds can add enough debug information to the instrumented Kuru executable that it exceeds the unchanged 128 MiB release-archive bound. The embedded installation acceptance must retain coverage instrumentation while packaging a representative bounded copy and must never modify an explicitly selected shipping artifact or Cargo's possibly hard-linked output.

## What Changes

- Prepare a private copied packaging input only for the implicit instrumented coverage fallback, removing debug symbols while retaining coverage instrumentation and profile output.
- Verify the original Cargo artifact remains byte- and identity-stable, and keep explicit `KURU_EMBEDDED_TEST_BINARY` inputs byte-for-byte unchanged.
- Retain the existing archive limit, installation, update, offline-memory, digest, and cleanup assertions.

## Impact

The change is limited to `apps/kuru-tui/tests/embedded_runtime.rs` and its focused native acceptance. It adds one bounded toolchain preprocessing step during coverage without changing product packaging, release profiles, archive limits, or runtime behavior.
