# Verification

## 1. A slow but progressing release download completes [critical]

- [x] 1.1 @regression (agent) `archive::download` and the published send/read path against a local fixture that trickles 30 chunks every 50 ms (about 1.5 s) with an injected 500 ms idle bound, and the same trickle against an old-shape client with a 400 ms flat total -> Observed 2026-10-03, macOS arm64: `slow_transfer_past_the_old_total_succeeds_within_the_idle_bound` and `slow_response_past_the_old_total_succeeds_within_the_idle_bound` receive every byte after more than 400 ms (3 of 3 runs); `the_old_flat_total_cuts_off_the_same_slow_transfer` reproduces the defect: the flat-total client fails mid-body with `error decoding response body: request or response body error: operation timed out`.
- [x] 1.2 @unit (agent) `release_client()` and `PublicGitHub::new` build the production clients -> Observed 2026-10-03: `the_release_client_builds_with_the_shared_bounds` and the existing `token_is_sent_only_on_api_metadata_requests` (which constructs `PublicGitHub::new`) pass.

## 2. A dead release download fails with its phase named [critical]

- [x] 2.1 @regression (agent) a fixture that sends headers and one chunk then stalls, with a 200 ms injected idle bound, run with and without the phase contexts -> Observed 2026-10-03, macOS arm64. Red with the four `.context` calls removed: `cargo test -p kuru-delivery --features tooling --lib download_timeouts` 3 passed, 7 failed; the stalled-body tests' messages were only `error decoding response body: request or response body error: operation timed out`. Green with the contexts: 10 passed in 3 of 3 runs (about 1.5 s each); messages name "read release asset body" / "read published release response body" and not the send phase.
- [x] 2.2 @integration (agent) a listener held without accepting (kernel backlog completes the handshake) and a closed port -> Observed 2026-10-03: red without contexts `error sending request ...: operation timed out` and `... tcp connect error: Connection refused (os error 61)`; green: both fail within the 5 s outer bound naming "send release asset request" / "send published release request". The unanswered case exercises reqwest 0.13.5's read timeout over `send()` (armed in `PendingRequest::poll`), not `connect_timeout`.

## 3. Existing delivery behavior is preserved

- [x] 3.1 @integration (agent) `mise run //packages/kuru-delivery:test` -> Observed 2026-10-03, macOS arm64: exit 0 in 83 s; lib 205 passed, every integration test binary passed (1 ignored, pre-existing).
- [x] 3.2 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run lint:tooling`, `mise run cospec -- validate --all --strict` -> Observed 2026-10-03, macOS arm64: each exits 0 (lint 101 s; root lint:windows ran every package's and kuru-tui's lint:windows, including kuru-delivery, 71 s; typecheck 53 s; lint:tooling 4 s; validate "0 errors, 0 warnings"). `cargo check -p kuru-delivery` without `tooling` also exits 0, confirming the constants' placement compiles for the shipped binary.
- [~] 3.3 @e2e (agent) a real `kuru --update` over a slow link against GitHub -> defer: needs a published release and a throttled network path; the injected-client tests drive the same `download`/`send`/`bounded_body` code that production calls.
