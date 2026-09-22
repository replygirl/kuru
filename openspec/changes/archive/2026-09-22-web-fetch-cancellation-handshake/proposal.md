## Why

The cancellation fixture used a fixed delay, so a slow runner could abort before the local fixture observed a complete request. The test must prove post-request cancellation without accepting an empty or partial request.

## What Changes

- Add a test-only bounded request-header readiness handshake to the web-fetch fixture.
- Make the cancellation regression wait for complete headers before aborting the fetch.

## Impact

- `packages/kuru-connectors/src/web_fetch.rs` test fixture only; no production transport behavior or CI topology changes.
