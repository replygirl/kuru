## 1. Package-owned debugger input

- [x] 1.1 Add `packages/kuru-delivery/support/prepare-windows-debugger.ps1`
  and `setup:windows-debugger` with the exact SDK installer URL, size, SHA-256,
  Authenticode publisher checks, x64 CDB containment checks, and observed CDB
  version/hash receipt.
- [x] 1.2 Wire debugger preparation only into the Windows
  connectors/core/platform shard and verify the isolation in
  `packages/kuru-delivery/tests/release_workflow.rs`.

## 2. Isolated stack-text capture

- [x] 2.1 Add the fixed `kc 80` and `!clrstack -n` capture to
  `packages/kuru-connectors/src/tools.rs` with explicit fixture-child selection,
  retained process identity, bounded output/waits, marker validation, and CDB
  cleanup before the caller's single existing target stop; replace the prior
  import control and verify the authoritative generated shell source equals its
  former no-import path.
- [x] 2.2 Add an owned known-sleep proof that runs once after the authoritative shell
  cases whenever CDB is configured, preserving the original 30-second outcome
  and limiting the selected parent watchdog to 75 seconds.

## 3. Evidence

- [x] 3.1 Pass focused formatter, connector and delivery typechecks, workflow
  validation, strict Cospec validation, and the actual apply gate.
  Focused parser, cleanup-marker, projection, delivery workflow, connector
  typecheck, connector clippy, actionlint, and tooling checks passed. The local
  Windows cross-check stopped in `aws-lc-sys` because the macOS host lacks the
  MSVC SDK headers, so it does not establish native behavior.
- [ ] 3.2 On native Windows, verify the pinned installer and observed CDB
  identity, known-sleep native and managed stacks, bounded cleanup, unchanged
  authoritative fixture outcome, and the connector shard's ordinary coverage
  receipt without treating diagnostic capture as a pass.
