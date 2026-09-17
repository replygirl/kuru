## Why

The persistent Windows activation-blocker fixture assumes the two-second recovery budget always permits a second native move. CI showed that the first checked no-move can itself consume the budget, so the fixture fails despite the intended bounded refusal and preserved stage.

## What Changes

- Keep the existing real Windows blocker, deadline, publication-error, staged-source, destination, and lock assertions.
- Require at least one observed checked no-move; the separate transient-blocker fixture continues to prove a successful retry after release.

## Impact

Only `packages/kuru-memory/src/provision/native_tests.rs` changes. Native Windows coverage time and production behavior are unchanged.

Observed old-head failure: PR39 commit `48c9283`, run `35183201262`, Windows memory-runtime job `105080063966` failed at the second-denial assertion after passing the recovery-budget assertion. The corrected native Windows run is pending the restacked push; macOS Rust format, memory typecheck and lint passed, but macOS cannot execute this `#[cfg(windows)]` fixture.
