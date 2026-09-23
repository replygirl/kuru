## Why

The private copied instrumented packaging probe currently invokes `--version`,
which exits through Clap before LLVM coverage data is reliably flushed on
Windows. The installed artifact itself remains valid, but the fixture cannot
establish the required coverage-profile receipt.

## What Changes

- Run the exact private copy with an isolated `config` command that returns
  normally and does not activate memory or a provider.
- Retain the source identity, copy bound, archive bound, and nonempty profile
  assertions in the existing embedded-runtime acceptance.

## Impact

Touches `apps/kuru-tui/tests/embedded_runtime.rs` and its focused native
fixture only. The explicit packaging-input override remains unchanged.
