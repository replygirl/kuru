# Proposal

## Why

Main fe70ad23 went red on macOS (run 37087605833, job 111101197086), and
fb2e9a37 failed the same way (run 36910091434). Both runtime fixtures closed all
three of their memory clients cleanly. The panic came from the next call, the
fixture's `await_managed_quiescence`, with "write memory service frame: Socket
is not connected (os error 57)". It does not come from `sibling.close()`, which
drops the client's streams and writes no frame. The change slug was chosen
before this diagnosis and keeps its original name.

The quiescence helper asks the owner to retire through maintenance
(`request_idle_retirement`). Here the owner was already retiring on its own
after its last client detached, which is the race the helper is designed to
wait behind. The maintenance hello write met the closing peer. On Darwin,
`uipc_send` checks `SS_ISCONNECTED` before `SS_CANTSENDMORE`. A peer disconnect
that lands while `sosend` has the socket unlocked is therefore reported as
ENOTCONN, while one that lands before the write is reported as EPIPE.
`is_peer_closed` accepted EPIPE, reset and EOF but not ENOTCONN. That made a
normal wait condition a hard error, against the project-memory-service
requirement that meeting a retiring owner is not an error in itself. An
electing client's handshake in `try_attach_observed` has the same gap.

## What Changes

- `is_peer_closed` also treats `io::ErrorKind::NotConnected` as the peer
  closing the connection. A stream whose connect succeeded loses its connection
  only to its peer. Maintenance then reads it as `RetirementReply::PeerClosed`,
  and an electing client reads it as `AttachMiss::PeerClosed`. Both wait for the
  owner lock exactly as they already do for EPIPE, reset or EOF.
- Every caller classifies only a connect or handshake, before any request is
  sent, or uses the result for owner-side attachment diagnostics. The
  uncertain-write fence does not consult this predicate, so request exchanges
  and mutation proof are unchanged.
- No retry, sleep or bound is added or changed. A live owner that keeps closing
  connections is still reported at the existing maintenance deadline, along
  with its peer-closed count.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. `project-memory-service` already requires that meeting a retiring owner
is not an error by itself; the implementation missed one Darwin errno.

## Impact

- `packages/kuru-memory/src/service.rs`: `is_peer_closed`, the `connect_miss`
  documentation, and a regression test that injects Darwin's handshake write
  failure.
- No API, protocol, configuration or documentation contract changes. The
  runtime fixtures and their teardown order are unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
