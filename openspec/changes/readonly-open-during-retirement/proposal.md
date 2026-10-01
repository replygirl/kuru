# Proposal

## Why

Since the project memory owner retires as soon as no client is attached, a CLI command's service is closing for a short window after the command process exits. A test fixture that then calls the direct `MemoryStore::open` read-only reads only the store's own Dolt endpoint record, which stays published until the retiring supervisor reaps Dolt, and borrows that Dolt without any lifetime guarantee. Main run 36789916109 (ubuntu-latest coverage partition 6) failed this way: `apps/kuru-tui/tests/preferences.rs` `an_explicit_framework_override_is_validated_against_its_own_part_budget` panicked at `preferences.rs:73` with `error communicating with database: expected to read 4 bytes, got 0 bytes at EOF`.

A second signature has the same cause (class (a), the removed 30 s idle window). `kuru_memory::test_support::await_managed_quiescence` failed with `connect to memory service for maintenance: Connection reset by peer (os error 104)` in PR #145's merge-commit run 36791870007, job 110146577721 (`kuru-runtime` `accounting_tests::ordinary_context_refuses_a_cursor_advanced_after_its_prior_summary_read`, which includes #142). That is the only observed occurrence: the task text also placed it on main job 110140432553, but that job's full log has no `reset by peer` and no `for maintenance` line. The context string is attached only to a `connect_local` error in `request_idle_retirement`. #142 maps a closed peer at the handshake to "no owner", but the connect step treats only NotFound and ConnectionRefused as "owner already gone". Inferred from tokio and kernel source, not observed on Linux: a connect queued in the listener backlog is reset when the owner drops its listener, and tokio's post-connect `take_error` returns ECONNRESET from the connect itself. On macOS the same race surfaces at the handshake as EPIPE (measured with a probe), which is already mapped, so it does not reproduce there. `try_attach_observed` has the identical connect match, so the client attach path carries the same gap (inferred from identical code, not observed).

The direct open consults no service authority; the owner lock and lifecycle lease are the authority and endpoint records are discovery hints. The product's managed read-only path (`open_managed_observed` through `attach_existing`) already waits on a held owner lock and decides local versus attached from the locks, so the defect is in the fixture's choice of API and in a false facade contract comment ("The project store lock excludes a live owner"; the store lock is a startup lock only). Before the owner stopped idling for 30 s the borrowed Dolt stayed alive, which masked this.

## What Changes

- The `Sandbox::preferences()` fixture in `apps/kuru-tui/tests/preferences.rs` inspects through the product path (`open_managed_observed`, read-only, after awaiting owner exit), the pattern `trust.rs` already uses, so it is ordered after the prior generation's Dolt reap and owner-lock release.
- Same-class read-only direct opens that run after a CLI command or TUI exit are converted the same way; the deliberate borrow of a live attached owner (fault injection at `terminal.rs` and the in-TUI polling loop) is kept.
- The `MemoryStore::open` facade documentation states the true contract: it consults no service authority, read-only borrows any live published Dolt endpoint without a lifetime guarantee, and callers must order it after owner exit, use `open_managed_observed`, or hold an attachment.
- A reset, broken pipe or EOF met while connecting to an owner that is retiring maps to "no owner" in `request_idle_retirement` (result `None`) and `try_attach_observed` (`AttachMiss::PeerClosed`), reusing `is_peer_closed`. `is_transport_unavailable` ("nothing is listening") is not widened. The caller then waits on the owner lock within its existing budget, as for a refused connect.
- A reset is not by itself evidence of a fault or of retirement: at the reset the service record is still present with the same generation and the owner lock is held, identical for a retiring and a live faulty owner. The owner lock decides. A retiring owner reaps Dolt and releases it, so the maintenance permit or electing client proceeds; a live faulty owner keeps it, so the maintenance permit fails at its existing deadline ("memory service owner is still active; maintenance cannot proceed") and an electing client at its existing readiness deadline. To keep a faulty owner from being hidden, `MaintenanceTrace` counts peer-closed outcomes apart from unanswered ones and the owner-still-active error names the counts.
- A deterministic `kuru-memory` test pins the contract the fixture relies on: a managed read-only open meeting a retiring owner waits for its reap and reads its own generation.
- Each signature gets its own tests: the retiring-owner inspection test, a maintenance connect queued before the listener drop, a client attach connect queued the same way, and a platform-independent routing unit test.
- No change to `store.rs` or `server.rs`, to the owner close order, or to `is_transport_unavailable`; no retry, sleep or raised deadline. The existing 100 ms re-request loop while the owner lock is held predates this change; only one transport outcome of it stops being fatal.

## Capabilities

### New Capabilities

### Modified Capabilities

The living specs (`project-memory-service`, `project-memory-owner`, `memory-store-lifecycle`) already require immediate shutdown after the last attachment, endpoint retirement before lock release, and cleanup before lock release; they say nothing wrong about read-only opens, attached inspection or endpoint records, so no delta spec is authored (the change also sets `skip_specs: true`). The service spec's requirement that a client arriving during shutdown has "no error caused only by the retiring owner" already covers the connect step; signature 2 is an implementation gap against it. Only the implementation (fixture, a code comment, and two connect-error mappings) was wrong.

## Impact

- `apps/kuru-tui/tests/preferences.rs` (fixture), other test fixtures found by the same-class audit (`terminal.rs`, `embedded_runtime.rs`, `packages/kuru-delivery/tests/support/mise_acceptance.rs`), `packages/kuru-memory/src/facade.rs` (doc comment), `packages/kuru-memory/src/service.rs` (new tests, the two connect-arm mappings in `request_idle_retirement` and `try_attach_observed`, and the `MaintenanceTrace` peer-closed count).
- No public API, configuration, dependency, or user documentation change; no product behavior change.
- Residual hazard recorded, out of scope: a new starter can elect in the gap between `attach_existing` returning no owner and the managed fallback's local open, so that reader borrows a serving owner's Dolt.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
