## 1. Deterministic close readiness

- [x] 1.1 Update the existing RPC close-order regression in
  `packages/kuru-connectors/src/rpc.rs` to observe an explicit real-peer ready
  handshake before sending `Close` and registering the first-reply waker.
- [x] 1.2 Preserve the existing first-wake admission/completion observation,
  public repeated-close result, and exactly-one-completed-peer assertions without
  changing production RPC behavior or deadlines.

## 2. Evidence

- [x] 2.1 Pass the focused RPC regression, connector typecheck, formatter,
  strict Cospec validation, and actual apply gate.
  All five focused RPC library tests passed after the real peer's ready frame
  was consumed. Connector typecheck, workspace formatting, and diff checks
  passed. Strict validation and the apply gate passed after this evidence update.
- [ ] 2.2 Pass the regression under the native Windows instrumented connector
  shard and emit its ordinary checked receipt.
