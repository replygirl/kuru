## 1. Shared fixture diagnostic

- [x] 1.1 Move the bounded staged-log capture into `kuru-memory` test support, retain `MemoryStore::temporary`'s fixture root until startup errors are annotated, and verify ordinary opens preserve their errors. Synthetic fixture diagnostic tests passed 2/2.
- [x] 1.2 Reuse the shared capture in the existing migration fixture and verify opt-in, exact stage selection, bounded output, and missing-log behavior with synthetic tests. The synthetic filter passed 2/2 and the affected real-Dolt migration test passed 1/1.

## 2. Focused verification

- [x] 2.1 Run the affected migration and runtime notes fixtures plus scoped memory/runtime typecheck, lint, formatting, and diff checks; record observed results. The runtime notes test passed 1/1; both package typecheck/lint tasks, Rust format check, and `git diff --check` passed. A sandboxed migration attempt could not reserve loopback; the identical host-access rerun passed. Native CI recurrence remains unexplained.
