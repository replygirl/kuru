## Why

The parallel-read runtime fixture replaces a retained file before releasing its test gate. Windows Pinned sharing can refuse that replacement, so a failed assertion can strand the checked read's blocking worker until the CI job timeout.

## What Changes

- Update `packages/kuru-runtime/src/tests.rs` to invalidate the prepared read with an extra hardlink, exercising its existing link-count refusal on every host.
- Release the test gate on every exit path, then retain the refused-read, accepted serial write, durable receipt, and no-replay assertions.

## Impact

One existing runtime test changes. The test uses bounded waits and should fail with a local diagnostic rather than consume the Windows shard's 90-minute limit.
