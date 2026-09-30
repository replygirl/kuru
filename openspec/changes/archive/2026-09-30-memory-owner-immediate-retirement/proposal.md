# Proposal

## Why

The per-project memory service owner keeps Dolt running for 30 seconds after its last client leaves (`SERVICE_IDLE_TIMEOUT`). The maintainer has decided, as final, that there is no idle window: the owner shuts down as soon as no client is attached and no accepted work is pending. The living specs require the idle interval, so the specs, not only the implementation, must change.

Removing the window exposes faults the grace period was hiding. A fresh owner could be retired by any other client (for example `kuru memory status`) that attaches and detaches between the starter's readiness polls, and the starter would then fail with "memory service exited before readiness". Cancelling a memory call (Esc) on a running client's only connection drops that connection, so the owner would retire under a running TUI and the next call would fail. Checked recovery drops its attachment to a verified successor before reopening, so that successor would retire and the reopen would elect yet another owner. A client that meets a retiring owner can fail on Windows with "private pipe connect timed out" or an unmapped `ERROR_PIPE_NOT_CONNECTED`, and maintenance can fail on a peer-closed handshake. Shutdown today also reaps Dolt before retiring the endpoint, which lets a client read an endpoint whose owner has already stopped serving.

## What Changes

- The owner has two empty states. Before its starter has attached it keeps serving other clients and waits for the starter, bounded by `memory.startup_timeout_secs` measured from endpoint publication; on expiry it exits cleanly with a warning. After its starter has attached it retires at once when the last attachment is released. `SERVICE_IDLE_TIMEOUT` is removed; the busy accept bound is kept as `OWNER_LOCK_RECHECK_INTERVAL` (35 s).
- The starter passes a random starter token to the owner it spawns as an optional tenth service argument and presents it in its handshake hello (an optional field, omitted when absent). Only that presentation marks the owner as reached. An owner started without a token (mixed versions) treats any authenticated attachment as reaching it. No `PROTOCOL_MINOR` bump. Unit 2 reuses the token to tag its activity record.
- Pending work is the set of authenticated attachments plus the write drain `MemoryStore::close` already performs. A dreaming client is an attached client. No owner-side work counter.
- Cancelled or failed calls on a writable session's primary attachment keep the old connection open, unused, until a replacement connection to the same generation has completed its handshake (Option B). The replacement starts at cancel time. No request is resent and the uncertain-write fence is unchanged. Read-only sessions get no hold and no replacement.
- Checked recovery keeps its attachment to the verified successor until `reopen_after_checked_recovery`, which reuses it.
- Shutdown order becomes: stop accepting, drop the listener, retire the endpoint, close the store (write drain, pools, supervisor and Dolt reap, lifecycle lease released at the reap), explicitly unlock the Owner lock, exit. Service locks are released with an explicit unlock.
- A client racing a retiring owner gets no error from that alone: it waits on the Owner lock and elects a successor within its unchanged startup budget. Windows peer-closed code 233 is mapped; the Windows pipe connect timeout on a busy pipe gains a typed marker checked only by the two election callers; maintenance maps a peer-closed handshake to "no owner". `attach_existing` probes the Owner lock only after a non-blocking Start acquire succeeds. The "did not publish a valid endpoint" error names a previous service still shutting down.
- Tests and fixtures stop relying on the idle grace; waits are event hooks, never sleeps.
- Deliberately unchanged: no deadline is raised, no retry is added to hide a fault, `startup_timeout_secs` keeps its meaning and range, isolation, ownership, recovery and the uncertain-write fence are not weakened.

## Capabilities

### New Capabilities

### Modified Capabilities

- `project-memory-owner`: "Attachment-bound idle cleanup" drops the 30-second wait, adds the not-yet-reached starter wait, reorders endpoint retirement before the reap, and requires a running writable client to keep an attachment across a cancelled request.
- `project-memory-service`: "Idle shutdown and safe recovery" drops the idle interval and reconnect-cancels-shutdown rule, reorders shutdown, and states that meeting a retiring owner is not by itself an error. The four shutdown scenarios ("Reconnect races idle shutdown", "Service or engine crash", "Maintenance after the final client exits", "Maintenance while a client remains active") are misplaced: they parse under "Reachable exact-ref candidate resolution" (confirmed by `cospec validate`, which counts 11 living scenarios there). Their wording must change, so that requirement is also MODIFIED, with its own text and its seven own scenarios verbatim and the four reworded in place, under their existing names. They cannot move under the shutdown requirement in this change: cospec 0.8.2's `archive/scenario-preservation` rule refuses a MODIFIED requirement that drops a scenario name, and offers no override.
- `memory-store-lifecycle`: "Explicit shared-store shutdown" replaces "after ... the idle interval expires" with immediate shutdown once the last client and accepted operation have drained.

## Impact

- `packages/kuru-memory/src/service.rs`, `service/rpc.rs` (confined to `Retirement`, `serve_attached` and the handshake), `store.rs` (`OpenOptions::starter_token`), `facade.rs`, `spawn_gate.rs`, `test_support.rs`, `test_support/served_owner.rs`.
- `packages/kuru-platform/src/windows/pipe.rs` and its tests (`NoFreeInstance`, `is_no_free_instance`).
- Tests in `apps/kuru-tui/tests/cli.rs`, `trust.rs`, `support/memory.rs`; comment in `packages/kuru-runtime/src/dream.rs`.
- Docs: `docs/memory.md`, `docs/install.md`, `docs/configuration.md`, `docs/development.md`, `apps/kuru-docs/guide/installation.md`, `docs/release.md` (mixed-version note).
- Service argument list gains an optional tenth argument; `ClientHello` gains an optional field. The pinned protocol surface is unchanged.
- User-visible cost: every command now starts a fresh owner (about 0.6 s to open an existing project instead of about 0.014 s to attach, maintainer figures); a command landing during a close waits for it inside the same startup budget.

## Surfaces

Interactive: the CLI and TUI see a new owner per command, a new wait-then-elect path, reworded errors, Esc no longer risking the service, and a read-only command that reports "run the command again" after its service retired. Deploy: the owner process's lifetime, its argument list and its shutdown order change on every platform, including a Windows-only pipe path. Integration is not checked: no third-party or external contract changes (the pinned service protocol is unchanged and Dolt is driven exactly as before). Agent behaviour is unchanged.

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
