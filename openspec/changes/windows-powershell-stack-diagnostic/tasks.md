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
- [x] 2.3 Place the fixed CDB command-file option before the retained target PID
  and project capture-rejection output through the existing terminal-safe
  redaction boundary with a 4 KiB cap and explicit truncation, without adding
  debugger commands or inspecting stores or unrelated processes. Exclude the
  target command line and debugger prompt-command echoes, and report explicitly
  when no projected output remains.

## 3. Evidence

- [x] 3.1 Pass focused formatter, connector and delivery typechecks, workflow
  validation, strict Cospec validation, and the actual apply gate.
  Focused parser, cleanup-marker, projection, delivery workflow, connector
  typecheck, connector clippy, actionlint, and tooling checks passed. The local
  Windows cross-check stopped in `aws-lc-sys` because the macOS host lacks the
  MSVC SDK headers, so it does not establish native behavior.
- [ ] 3.2 On native Windows, verify the pinned installer and observed CDB
  identity, known-sleep native and managed stacks, bounded cleanup, unchanged
  authoritative fixture outcome, bounded projected rejection evidence, and the
  connector shard's ordinary coverage receipt without treating diagnostic
  capture as a pass.
- [x] 3.3 Pass the focused CDB parser/projection checks, connector typecheck and
  clippy, formatter, strict Cospec validation, and actual apply gate after the
  option-order and rejection-diagnostic refinement.
  The focused CDB library filters passed 2/2 and exercise accepted stack text,
  incomplete and unsafe output, projected failure text, redaction, exclusions,
  explicit empty output, and the combined 4 KiB bound. Connector typecheck,
  connector clippy with warnings denied, workspace formatting, and diff checks
  passed. Strict validation and the apply gate passed after this evidence update.
