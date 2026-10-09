# Proposal

## Why

Kuru's typed doctor/headless/canary statuses immediately terminate from `finish_dispatch`, after authority cleanup. On Windows Rust's process exit uses ExitProcess, bypassing the ordinary C-runtime return path on which LLVM registers its profile writer; existing doctor error cases pass but their exercised branches are missing from the saved native coverage report.

## What Changes

- Return the existing typed status as Rust's standard ExitCode through both main entrypoints, preserving diagnostics and ordinary error reporting.
- Retain dispatch worker joining, checked authority cleanup and prompt termination with blocked stdin/stdout-only threads. Verify existing native CLI acceptance and error-path profile collection without explicit profile flushes.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`apps/kuru-tui/src/main.rs` and contributor coverage guidance only. No CLI status/message, runtime authority, dependency, threshold or instrumentation API changes. Valid native Windows before/after exports confirm previously absent invocation-error counters from the same existing cases; verification names the observed groups.

## Surfaces

- [x] interactive — CLI exit status and diagnostics
- [ ] deploy
- [x] integration — Rust main termination and LLVM exit handlers
- [ ] agent-behavior
