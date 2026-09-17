## 1. Denied private-stage child reconciliation

- [x] 1.1 Accept a rejected private-child removal reporting native raw error 5 into the existing bounded cleanup window in `packages/kuru-memory/src/files.rs`, keeping the rejected OS32 and uncertain cases, the exact identity checks and the first-cause exhaustion error unchanged.
- [x] 1.2 Add a Windows-only regression fixture holding a delete-on-close child so the first checked removal is rejected with native error 5, released after the first recoverable result.
- [x] 1.3 Cover both reported phases for native error 5 in the `provision/native_tests.rs` cache-invalidation fixture, keeping its identity check and its two-second bound.

## 2. Verification

- [x] 2.1 Run scoped formatting, memory typecheck and lint, and the host-runnable provision and files fixtures; record Windows execution as pending until native CI.
- [x] 2.2 Archive the change before the branch commit.

Observed: `mise run format:rust:fix` and `mise run format:check` clean; `mise run //packages/kuru-memory:typecheck` and `:lint` passed; `cargo test -p kuru-memory --lib -- provision::native_tests files::tests` passed 12/12 on macOS with the shared verified bundle mirror. The new fixture and the changed predicate are `#[cfg(windows)]`; a Windows-target `cargo check` of `kuru-memory` is not possible on this macOS host because `libsqlite3-sys` needs an MSVC C toolchain, so the Windows-only paths were cross-checked as an isolated `--target x86_64-pc-windows-msvc` snippet and otherwise remain pending native CI.
