## 1. Delete-pending stage child fixture

- [x] 1.1 Leave the blocking child genuinely delete-pending in `delete_pending_private_file`, retaining the handle that keeps the name while a separate delete-on-close handle applies the disposition, and verify the fixture's own assertion that a later ordinary open is refused with native error 5.
- [x] 1.2 Explain in the outcome assertion of `files::tests::private_temp_retries_a_denied_child_delete_after_the_holder_releases` what an uncertain result means instead, and verify the test names the completed child delete and the still-occupied parent when it fails.
- [x] 1.3 Keep the recovery predicate, the two-second bound, the retry spacing, the exact identity checks and the first-cause exhaustion error unchanged, and verify by diff that only the fixture, its assertions and the exhaustion context differ.

## 2. Bounded cleanup exhaustion diagnostics

- [x] 2.1 Count reconcile attempts through `close_windows_private_stage_with` and report that count with the actual elapsed window in the `wait_for_cleanup_retry` exhaustion context, and verify the context still carries the retained first cause.

## 3. Verification

- [x] 3.1 Run scoped formatting, memory typecheck and lint and the host-runnable `files::tests` and `provision::native_tests` fixtures, and verify each reports clean; record Windows execution as pending until native CI.
- [x] 3.2 Type-check the changed Windows-only code against `x86_64-pc-windows-msvc` and verify the result, recording that a full cross-target check of `kuru-memory` is impossible on this host.
- [x] 3.3 Archive the change before the final branch commit and verify the archive directory exists.

Observed: the fixture now opens a retained `GENERIC_READ` handle sharing read/write/delete and applies the disposition with a separate delete-on-close handle, and asserts its own premise with an ordinary `File::open` that must report raw error 5. `mise run format:rust:fix` and `mise run format:check` clean; `mise run //packages/kuru-memory:typecheck` and `:lint` passed; `cargo test -p kuru-memory --lib -- files::tests provision::native_tests` passed 12/12 on macOS with the shared verified bundle mirror. `cargo check --target x86_64-pc-windows-msvc -p kuru-memory --lib --tests` is impossible on this host (`libsqlite3-sys` compiles `sqlite3.c` for the MSVC target with no MSVC C toolchain present), so the changed `#[cfg(windows)]` fixture and exhaustion reporting were type-checked as an isolated `rustc --target x86_64-pc-windows-msvc --edition 2024 --emit=metadata` snippet, which compiled clean. The reviewed diff touches only the fixture, its assertions, the attempt counter and the exhaustion context. Windows execution of the regression rows stays pending native CI.
