## Why

The RPC close-order regression can send `Close` before the real peer task has
entered its receive loop, so native instrumented scheduling can fail the test
without violating the already-specified close-admission ordering.

## What Changes

- Add an explicit real-peer readiness handshake to the existing deterministic
  close-order regression before it sends `Close` and observes the first reply
  wake.
- Keep the production close path and its admission, completion, repeated-close,
  and one-completed-peer assertions unchanged.

## Impact

Touches only the RPC regression in `packages/kuru-connectors/src/rpc.rs`; test
runtime and production behavior are unchanged.
