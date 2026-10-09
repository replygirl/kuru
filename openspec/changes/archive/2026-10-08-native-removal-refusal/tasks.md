# Tasks

## 1. Native file removal refusal

- [x] 1.1 Add the actual nonwritable-parent removal contract in `packages/kuru-platform/src/fs/unix.rs`; restore retained permissions before observing assertions and verify exact rejection metadata, preserved file/adjacent state and a successful retry. Simplify `src/unix.rs` listener regression observations while retaining every assertion and explicit child cleanup before assertions.
- [x] 1.2 Run platform behavior with the pinned 95% coverage gate, formatting and lint; record actual results and any root-user DAC limitation, then archive before the follow-up branch commit. All native PR/main gates remain required before merge and delivery.

Final macOS platform behavior passes 146/146. The actual pinned 0.9.1 owning
coverage task and independently gated JSON report both pass: 5947/6243 lines
(95.258690%), with the unchanged canonical metric and source inventory. Rust
formatting and final platform lint pass. The unlink case actually observed
native PermissionDenied as a non-root account, restored retained permissions,
preserved exact identity/bytes/adjacent state, then completed the checked retry.
Root bypasses that DAC boundary, so a root run prints that limitation and cannot
establish the refusal. The listener regression retains all observations and
asserts them after explicit child reap and reader join; no synthetic I/O errors
are constructed for failed assertions.

Native PR/exact-main checks remain external delivery gates after local source
archive. Windows standalone coverage still needs further expansion under the
enclosing goal; this Unix-only test does not claim to cure that gate.
