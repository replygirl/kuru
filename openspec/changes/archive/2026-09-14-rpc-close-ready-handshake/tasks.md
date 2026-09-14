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
- [x] 2.2 Pass the regression under the native Windows instrumented connector
  shard and emit its ordinary checked receipt.
  Exact-head run `34804953102`, job `103854899023`, passed all 135 connector
  tests, including the real-peer ready-frame close regression and its existing
  bounds and cleanup cases. Artifact `10332707571` contains the ordinary
  connector receipt (SHA-256
  `027af5aa66504beb418ac32223abcfff91fcc35af8066413c2a41595e3fcc2c9`),
  which binds synthetic source `ca0b858f43cb4112199dd927e8332aa68aaae9c0`
  to tree `a3eb835f1d009c364c2d1cc9c5e135d8528fca0f` with 15 selected
  executables successful and 49 explicitly omitted.
