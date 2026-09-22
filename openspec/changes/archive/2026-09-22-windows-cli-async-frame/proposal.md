## Why

Native Windows coverage runs of the `kuru` executable have produced a main-thread stack overflow in ordinary CLI child processes after their test scripts completed. The same failure appeared on two branches with different feature changes; a later diagnostic that materialized and measured the CLI future made many more commands overflow before its first marker, so that probe cannot establish a shell-specific cause. The binary's Tokio entry currently holds the async CLI and internal dispatch futures inline in one generated main future, making its stack layout sensitive to unrelated branch growth under instrumentation.

## What Changes

Heap-pin the existing async binary dispatch once at the Tokio entry. Preserve all branch selection, awaits, errors, cancellation, and cleanup exactly; remove transient diagnostic probes from the proposed production fix. Use the existing failing native Windows CLI and terminal cases as behavioral proof rather than raising stack sizes or adding a parallel diagnostic runner.

## Capabilities

### New Capabilities

### Modified Capabilities

The living requirements already require native Windows execution; this is an implementation correction with no spec contract change.

## Impact

`apps/kuru-tui/src/main.rs` is the only production source change. No public API, configuration, dependency, persisted data, or CLI output changes. The entry incurs one dispatch-future heap allocation per process; its async work and runtime remain the same.

## Surfaces

- [x] interactive — CLI and TUI startup on Windows
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
