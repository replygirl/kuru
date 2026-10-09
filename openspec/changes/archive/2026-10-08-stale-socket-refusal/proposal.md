# Proposal

## Why

CI37865819250 Ubuntu partition4 observed ConnectionReset where the stale-endpoint fixture required ConnectionRefused. Dropping a listening socket in the parent does not establish a non-listening endpoint while concurrent forks may retain a native copy.

## What Changes

- In `packages/kuru-platform/src/local_ipc.rs`, bind a native stream socket without listening, then close it under the existing spawn lock before the bounded connection attempt.
- Keep exact ConnectionRefused, failed occupied-name binding, unchanged socket identity and adjacent bytes; add no accepted error variants or timeout changes.

## Impact

One existing Unix test fixture only; no production API, dependency, coverage filter or workflow changes. Linux documents refusal for a non-listening endpoint and reset for peer closure: https://man7.org/linux/man-pages/man7/unix.7.html. The observed failure does not prove which concurrent child retained the listener.
