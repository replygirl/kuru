## 1. Native Windows discriminator

- [x] 1.1 Add a `packages/kuru-platform/src/fs/windows.rs` fixture that opens separate fresh private directories with the current metadata rights, metadata plus `FILE_TRAVERSE`, and metadata plus `FILE_LIST_DIRECTORY`; for each, verify exact directory identity and child bytes around an explicit DELETE open and an independent pathname rename, and report only fixed outcomes and OS codes.
- [ ] 1.2 Run the exact fixture on native Windows with normal reviewed publication checks; record each open, DELETE-open and rename result, then classify whether any tested rights actually pin the name before proposing a product access-mode change.
