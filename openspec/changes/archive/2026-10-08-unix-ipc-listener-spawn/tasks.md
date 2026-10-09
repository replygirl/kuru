# Tasks

## 1. Serialize private listener creation with owned spawns

- [x] 1.1 Add causal native bind/connect/accept regressions that fail before serialization and pass afterward, retaining real endpoint refusal and an actual owned child.
- [x] 1.2 Use the existing platform spawn lock during private Unix socket creation, release it before Pending/await, and route native fixtures through the checked operations; preserve waiter semantics, checked socket identity, modes, and cleanup.
- [x] 1.3 Document the bounded internal listener guarantee and run focused platform behavior, formatting, and lint; record observed evidence and retain native PR/main coverage gates as delivery requirements.

All three causal regressions failed against original creation (exit 101,
observed admission event false) and pass after correction, including real
owned-child stale-endpoint refusal and byte exchange. Final platform behavior
passes 145/145, including independent pending accept waiters, stale-parent
refusal before dispatch and native missing/nonexecutable spawn refusal followed
by a complete piped successor. Final platform lint, formatting and public docs
checks pass. Independent review confirms no await retains the lock and the
existing accept future's waiter semantics remain intact.

The first coverage report was contaminated by old workspace binaries. A fresh
same-pinned-tool target reports 5897/6210 (94.959742%); the unchanged local 95%
gate fails and is not claimed as accepted. Coverage expansion remains active
under the enclosing goal. Native PR/exact-main checks and every existing 95%
gate remain required before merge and goal completion; local behavior is not
proof of native Windows execution or final coverage acceptance.
