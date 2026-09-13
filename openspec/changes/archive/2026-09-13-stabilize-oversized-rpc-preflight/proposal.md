## Why

The oversized RPC dispatch regression uses a sleeping peer even though preflight
must reject the payload before it writes the wire. A macOS coverage run reported
unconfirmed cleanup in that sleeping fixture; its cause remains undetermined.

## What Changes

- Replace the final oversized-dispatch fixture with an initialized, idle peer
  and assert that no request reaches its transcript while close is confirmed.
- Preserve the existing timed request and retained-cleanup cases unchanged.

## Impact

Touches only the connector RPC test and its test-change record. It adds no
production behavior, timeout, retry, or CI-time policy.
