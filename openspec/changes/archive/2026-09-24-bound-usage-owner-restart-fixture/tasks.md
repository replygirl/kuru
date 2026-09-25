## 1. Restart fixture

- [x] 1.1 Label initial owner open, close, endpoint and lock release, and successor open failures; verify fixed phase contexts and unchanged usage assertions.
- [x] 1.2 Serialize test-only close through successor admission against unrelated spawns; verify the existing gate cannot be reacquired on that path.
- [x] 1.3 Run the focused real-Dolt restart fixture and owning memory lint/format; record macOS final-head CI as a post-push merge gate until observed.

Observed locally: the focused pinned-Dolt fixture passed 1/1 in 3.93 seconds, and owning memory Clippy passed. The first sandboxed attempt failed before owner startup because private loopback binding was denied; the native rerun passed. Final-head macOS memory CI is unrun and required before merge. The prior hosted failure lacked a phase label, so its cause remains unproven.
