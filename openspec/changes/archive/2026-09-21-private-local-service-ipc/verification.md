## 1. Successive private local clients [critical]

- [x] 1.1 @integration (agent) run real Unix socket clients through one listener in sequence -> `mise run //packages/kuru-platform:test` passed on macOS after sandbox escalation on 2026-09-21; four `local_ipc` tests passed, including distinct first and second byte payloads
- [~] 1.2 @integration (agent) run the native Windows named-pipe two-client and cancelled-accept fixture -> defer: local host is macOS; the PR's native Windows CI must confirm distinct payloads and timed-out accept recovery before merge

## 2. Checked endpoint privacy and ownership [critical]

- [x] 2.1 @integration (agent) exercise Unix public parent, non-socket, group-readable socket, occupied name and replaced-name fixtures -> `mise run //packages/kuru-platform:test` passed all four `local_ipc` tests on macOS after sandbox escalation on 2026-09-21
- [~] 2.2 @integration (agent) exercise Windows duplicate first-instance bind and private DACL through the native fixture -> defer: local host is macOS; the PR's native Windows CI must confirm occupied-name rejection before merge

## 3. Package-only compatibility

- [x] 3.1 @regression (agent) compile the package and all native targets with lint -> `mise run //packages/kuru-platform:lint` and `mise run //packages/kuru-platform:test` passed on macOS on 2026-09-21 (15 unit, 20 filesystem, 4 local IPC and 2 Unix process-group tests)
- [x] 3.2 @regression (agent) cross-check Windows test targets -> `mise exec -- cargo check -p kuru-platform --target x86_64-pc-windows-msvc --features test-support --tests` passed on macOS on 2026-09-21; this is compilation evidence, not native execution
- [x] 3.3 @regression (agent) obtain independent bounded-diff review -> config worker found missing Unix test deadlines; fixed named 5-second Unix and 8-second Windows exchange bounds, reran four real Unix tests, and reviewer reported no remaining scoped finding on 2026-09-21
