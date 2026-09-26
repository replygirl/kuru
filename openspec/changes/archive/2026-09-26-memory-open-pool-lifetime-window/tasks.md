## 1. Lifetime-safe opening window

- [x] 1.1 Build `Server::pool` pools with the ordinary lifetime `acquire_timeout` via `connect_lazy_with` and bound only the first acquire (retry on `PoolTimedOut` until the opening deadline, exit on identity rejection); verify with 1.3.
- [x] 1.2 Make the test seam stall callbacks until one instant from the first callback; verify the existing probe test and the opening regression test still pass.
- [x] 1.3 Add a retained-pool test asserting the ordinary lifetime window and contended ordinary acquisition after `Ready`; verify it fails on the unfixed head and passes after.

## 2. Verification

- [x] 2.1 Run the full kuru-memory test task, the two originally failing kuru-runtime tests, fmt and clippy; record results.

## Observed evidence

Recorded in `verification.md` (macOS aarch64 host, 2026-09-26). Not run: native Windows/Linux CI, the workspace coverage gate, and the full kuru-runtime suite.
