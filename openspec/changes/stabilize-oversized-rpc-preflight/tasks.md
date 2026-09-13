## 1. Oversized RPC preflight fixture

- [x] 1.1 Update `packages/kuru-connectors/src/rpc.rs` so the final oversized
      dispatch case consumes an incoming ready notification before using an idle
      read step, then verify the oversized message produces no outbound wire record.
- [x] 1.2 Run the exact RPC bounds test and verify it rejects the oversized
      dispatch while `Rpc::close` confirms cleanup without changing the existing
      timed-request or retained-cleanup cases. *(Observed: 2026-09-13,
      instrumented pinned `cargo-llvm-cov 0.9.1 llvm-cov test -p kuru-connectors --lib
      rpc::tests::rpc_bounds_protocol_failures_timeout_and_oversized_dispatch
      --locked --no-report` passed 1 test with 154 filtered using fresh isolated
      `target-rpc-llvm-091`; 25 raw profiles remained there and no LLVM profile
      error was observed.)*
