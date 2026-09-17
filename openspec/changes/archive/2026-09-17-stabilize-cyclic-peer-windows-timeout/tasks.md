## 1. Cyclic peer regression

- [x] 1.1 Update `packages/kuru-runtime/src/tests.rs` with the bounded outer safety timeout and diagnostic, and verify the existing semantic assertions remain intact.
- [x] 1.2 Run the focused regression and required runtime static checks, then record the observed results: `cyclic_peers_stop_at_peer_round_limit` passed 1/1; runtime typecheck, lint, and Rust format check passed.
