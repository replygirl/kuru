# Tasks

## 1. Regression test

- [x] 1.1 Add `a_handshake_write_disconnected_by_a_retiring_owner_is_a_peer_closed_miss`, which runs the real `connect_handshake` over a stream whose write fails with ENOTCONN, and verify it fails before the fix with "write memory service frame: not connected" (the CI chain)

## 2. Fix

- [x] 2.1 Classify `io::ErrorKind::NotConnected` as peer closed in `is_peer_closed` and document why and where it applies; verify the regression test and the existing peer-closed tests pass
- [x] 2.2 Confirm every `is_peer_closed` caller is a connect, a handshake or a diagnostic, and that the uncertain-write fence does not consult it; verify by grep

## 3. Checks

- [x] 3.1 Run `mise run //packages/kuru-memory:test` and verify it passes
- [x] 3.2 Run the two affected runtime fixtures five times each and `kuru-runtime` `accounting_tests`, and verify they pass
- [x] 3.3 Run `format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check` and `cospec validate --all --strict`, and verify they pass
