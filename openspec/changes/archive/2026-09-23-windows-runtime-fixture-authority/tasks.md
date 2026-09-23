## 1. Runtime fixture authority

- [x] 1.1 Use one canonical retained project root in `packages/kuru-runtime/src/review_tests.rs` and verify the accepted file mutation cancellation fixture reaches its checkpoint and no-replay assertions (host fixture passed 1/1 in 3.24s).
- [x] 1.2 Equip `packages/kuru-runtime/src/windows_tool_tests.rs` with an isolated private checkpoint store so native replay exercises the specified file authority and durable receipts; independent source review found no blocker.
- [x] 1.3 Run the runtime all-target check (green in 37.74s) and record native Windows execution as pending hosted CI; local cross-check reached native C dependencies but cannot compile without the Windows SDK headers.
