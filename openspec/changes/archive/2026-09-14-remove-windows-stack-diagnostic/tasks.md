## 1. Remove temporary diagnostics

- [x] 1.1 Remove the connector CDB-only selector, capture helpers, metadata,
  known-sleep control, and armed fixture allowances while preserving the
  stock-module bootstrap, RPC readiness regression, ordinary Windows shell
  fixtures, cleanup, and normal deadlines; verify with focused connector tests,
  typecheck, Clippy, formatting, and a source diff against the proven head.
- [x] 1.2 Remove the debugger preparation script and package task, the
  connector-shard SDK setup, and debugger-only workflow assertions while
  preserving the four shards, receipt aggregation, install job, and conditional
  native gate; verify with focused delivery workflow tests, actionlint, tooling,
  formatting, and diff checks.

## 2. Delivery evidence

- [x] 2.1 Pass strict Cospec validation and the actual apply gate, record the
  focused cleanup evidence, and archive this completed chore before publishing
  the cleaned source for the required exact-head native CI matrix.
  Connector and delivery all-target/all-feature typecheck and Clippy passed,
  as did full Rust formatting, actionlint, tooling lint, diff checks, the
  ordinary cleanup-marker regression, and the Windows shard fail-closed
  workflow test. The broader all-feature release-workflow target passed all
  native/workflow cases but retained four pre-existing local Cocogitto fixture
  failures; no source was changed for them. Strict validation and the actual
  apply gate passed before cleanup. The final exact-head matrix remains a PR
  acceptance gate rather than evidence for this not-yet-published source.
