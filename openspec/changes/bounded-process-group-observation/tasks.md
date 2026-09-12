## 1. Bounded Unix observer

- [x] 1.1 Update the private Unix post-cleanup observer in `packages/kuru-delivery/src/command.rs` to poll `EPERM` only through its existing five-second deadline and to accept only `ESRCH`, without sending another kill after root reap or changing Windows/generic capture/MCP semantics.
- [x] 1.2 Preserve existing `exists`-to-`ESRCH` success and unexpected-observer-error failure behavior, with the final bounded permission diagnostic identifying the failed observation.

## 2. Regression coverage

- [x] 2.1 Add deterministic private observer-seam cases for `EPERM` then `ESRCH`, persistent `EPERM`, existing then `ESRCH`, and an unexpected error; verify the transient-permission case fails before the correction and passes after it.
- [x] 2.2 Retain and run the real owned silent-descendant fixture plus all focused `bounded_output` cases, verifying termination and reaping remain observable without a second kill.

## 3. Validation

- [x] 3.1 Run the delivery package's focused lint and typecheck tasks and record observed command results; name native Windows execution as unrun locally.
