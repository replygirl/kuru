# Tasks

## 1. Regression first

- [x] 1.1 Add `bundle::tests::dropped_lock_is_released_despite_an_inherited_duplicate` and verify it fails with `WouldBlock` on unfixed code (output saved to `red.log`)
- [x] 1.2 Measure the `bundle::` module 20 times at default threads with `isolate_lock_release!` stripped on unfixed code and verify failures are observed

## 2. Explicit release

- [x] 2.1 Add the `HeldLock` guard (explicit `File::unlock` before close on every exit path, `Deref<Target = File>`) and return it from `Directory::lock`; verify drop order keeps stage removal before lock release
- [x] 2.2 Use the guard for the Windows installation lease in `update.rs` and verify every use site is a borrow or a discarded `_lease`
- [x] 2.3 Make `assert_clean` and `retained_lock` unlock explicitly and remove `isolate_lock_release!`/`delegated_to_lock_child`; verify no test assertion, timeout or retry changed

## 3. Verification

- [x] 3.1 Verify the regression test passes, `bundle::` 20 times at default threads and the partition-3 trio 20 times show 0 failures
- [x] 3.2 Run `//packages/kuru-delivery:test`, `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check` and strict cospec validation and record observed evidence
- [x] 3.3 Record the workspace audit of release-by-close locks with follow-ons (file:line) in the proposal's Impact section
