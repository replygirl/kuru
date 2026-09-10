## Why

The native connector fixture occasionally exits before `main`, failing existing
RPC/MCP assertions with EOF: a bounded macOS reproduction observed two SIGKILLs
in 480 hardlink launches, and exact-PID system logs attributed both to Gatekeeper
after a retired sibling alias could no longer be inspected. Equivalent symlink
and stable-path controls each completed 480 launches without failure.

## What Changes

- In `packages/kuru-connectors/src/test_support.rs`, use Unix symlinks to the
  immutable compiled peer while retaining independent invocation paths, wire
  plans, transcripts and reference-counted cleanup.
- Exercise both alias and direct invocation and assert the running peer reports
  the stable executable identity. Preserve concurrent-plan isolation and the
  existing sibling-retirement/final-owner cleanup regression.
- Keep native Rust fixtures and all production protocol behavior unchanged;
  add no retries, sleeps, security exceptions or stochastic gate stress tests.

## Impact

Only connector test support and this change record are affected. The deterministic
identity assertion adds one bounded native peer exchange; there are no new
dependencies, runtime configuration changes or coverage exclusions. Diagnostic
details remain outside published documentation in
`/tmp/kuru-connector-startup-diagnosis.md`.
