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
- [x] 2.4 Remove the native-incompatible detach-on-exit option while preserving
  noninvasive attachment, the fixed command file's explicit `qd`, bounded CDB
  cleanup, and every existing authoritative test outcome and deadline.

## 3. Evidence

- [x] 3.1 Pass focused formatter, connector and delivery typechecks, workflow
  validation, strict Cospec validation, and the actual apply gate.
  Focused parser, cleanup-marker, projection, delivery workflow, connector
  typecheck, connector clippy, actionlint, and tooling checks passed. The local
  Windows cross-check stopped in `aws-lc-sys` because the macOS host lacks the
  MSVC SDK headers, so it does not establish native behavior.
- [x] 3.2 On native Windows, verify the pinned installer and observed CDB
  identity, known-sleep native and managed stacks, bounded cleanup, unchanged
  authoritative fixture outcome, bounded projected rejection evidence, and the
  connector shard's ordinary coverage receipt without treating diagnostic
  capture as a pass.
  Run `34801590851` captured accepted native and managed stack text from the
  authoritative failing ToolHost fixture, passed the known-sleep control, and
  preserved the original shell failure as decisive. Runs `34800141435` and
  `34801590851` retained bounded projected startup rejection and successful
  capture evidence without changing the authoritative outcome. Exact-head run
  `34804953102`, job `103854899023`, verified the pinned installer and observed
  CDB identity, passed the always-run known-sleep capture without cleanup
  uncertainty, passed all 135 connector tests, and emitted artifact
  `10332707571`. Its checked connector receipt has SHA-256
  `027af5aa66504beb418ac32223abcfff91fcc35af8066413c2a41595e3fcc2c9`,
  binds synthetic source `ca0b858f43cb4112199dd927e8332aa68aaae9c0`
  to tree `a3eb835f1d009c364c2d1cc9c5e135d8528fca0f`, and records 15
  selected executables successful, 49 omitted, and 58 profiles. The bootstrap
  made the authoritative shell case pass in the final run, so failure capture
  did not execute there and did not contribute to that pass.
- [x] 3.3 Pass the focused CDB parser/projection checks, connector typecheck and
  clippy, formatter, strict Cospec validation, and actual apply gate after the
  option-order and rejection-diagnostic refinement.
  The focused CDB library filters passed 2/2 and exercise accepted stack text,
  incomplete and unsafe output, projected failure text, redaction, exclusions,
  explicit empty output, and the combined 4 KiB bound. Connector typecheck,
  connector clippy with warnings denied, workspace formatting, and diff checks
  passed. Strict validation and the apply gate passed after this evidence update.
- [x] 3.4 Pass the focused CDB parser/projection checks, connector typecheck and
  clippy, formatter, strict Cospec validation, and actual apply gate after the
  detach-on-exit correction. The preceding native run remains failure evidence:
  CDB emitted `The system does not support detach on exit`, no fixed marker or
  known-sleep capture was accepted, the authoritative ToolHost still timed out
  before `Join-Path`, and no connector receipt was emitted.
  After removing only `-pd`, the two focused CDB library tests, connector
  typecheck, connector clippy with warnings denied, workspace formatting, and
  diff checks passed. Strict validation and the apply gate passed after this
  evidence update; native capture and the checked shard receipt remain task 3.2.
