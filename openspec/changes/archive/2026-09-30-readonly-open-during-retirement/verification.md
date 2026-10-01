# Verification

All local evidence below is macOS arm64 (Darwin 27.0.0), debug, 2026-09-30, unless stated. Nothing ran on Windows or under ubuntu coverage instrumentation.

## 1. A read-only inspection after owner retirement reads its own generation [critical]

- [x] 1.1 @regression (agent) `managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation` against a real owner paused after endpoint retire and after reap -> passed. Red measured by temporarily replacing the managed open with the direct `MemoryStore::open`: failed with "the managed inspection opened while the retiring owner held its lock: Ok(())" (not committed). The test's own control, a direct open during the pause, reads the retiring Dolt and then fails its first read after the reap.
- [x] 1.2 @integration (agent) `cargo test -p kuru --test preferences` (3 tests, both `preferences()` callers included) -> 3 passed; `cargo test -p kuru --test terminal terminal_selections_survive_restarts_picker_changes_and_failed_database_writes` (the converted terminal `preferences()` path) -> passed; `mise run //apps/kuru-tui:test:embedded-runtime` (converted transcript reopen) -> 5 passed. The full `//apps/kuru-tui:test` aggregate was not run.
- [x] 1.3 @runtime (agent) `an_explicit_framework_override_is_validated_against_its_own_part_budget` repeated 10 times -> 10 of 10 passed (the unforced baseline was already 20 of 20, so this alone does not prove the fix)

## 1b. A reset met while connecting to a retiring owner is "no owner" [critical]

- [x] 1.4 @regression (agent) `maintenance_connect_queued_before_the_listener_dropped_waits_for_the_owner_lock` -> passed on macOS, and also passed against the pre-fix mapping, as expected: Darwin completes the queued connect and the handshake meets the closed peer. The ubuntu leg is the one that exercises the connect-reset arm
- [x] 1.5 @regression (agent) `a_client_connect_queued_before_the_listener_dropped_is_a_peer_closed_miss` -> passed on macOS (also passes pre-fix there, same reason), including the successor election after the owner lock release
- [x] 1.6 @unit (agent) `a_connect_reset_by_a_retiring_owner_is_a_peer_closed_miss` (`connect_miss`: reset -> PeerClosed with and without the maintenance context, refused -> TransportUnavailable, permission denied and the connect deadline -> fault) -> passed; red against the pre-fix mapping (no peer-closed arm): `left: None, right: Some(PeerClosed)`
- [x] 1.7 @unit (agent) `maintenance_names_a_live_owner_that_keeps_closing_connections` (owner lock held, published listener that accepts and drops, `startup_timeout_secs = 1`) -> passed 6 of 6; red against the pre-fix trace and messages: "the closed requests were not counted as peer-closed: memory maintenance owner-response deadline exceeded: deadline has elapsed". The deadline falls either between requests or inside one, so both deadline errors now carry the trace
- [x] 1.9 @runtime (agent) Linux kernel mechanism probe (Perl, AF_UNIX, non-blocking connect queued on a listener that then closes; OrbStack Linux 7.0.14 kernel) -> `connect=ok`, `SO_ERROR=104 (Connection reset by peer)`; same probe on Darwin -> `SO_ERROR=0`, write EPIPE. That tokio's connect returns this SO_ERROR via `take_error` is from tokio source, not run on Linux
- [~] 1.8 @runtime (agent) native ubuntu and Windows legs of 1.4 and 1.5 -> defer: the kuru tests were not run on Linux or Windows locally; the ubuntu CI leg is their verification of the connect arm

## 2. Contract and same-class sites

- [x] 2.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check` -> all exit 0; `mise run //packages/kuru-memory:test` -> exit 0 (lib 421 passed, 4 ignored); `kuru-runtime` `ordinary_context_refuses_a_cursor_advanced_after_its_prior_summary_read` -> passed
- [x] 2.2 @manual (human) review that `terminal.rs` live-owner borrows are unchanged -> reviewed in the independent PR review (no code read changed): `InvalidSessionCatalogMode::inject` still uses the direct read-only open, unchanged

## 3. Platforms

- [~] 3.1 @runtime (agent) native Windows and ubuntu coverage runs -> defer: no Windows evidence was collected locally; CI is the only authority for ubuntu coverage timing. `packages/kuru-delivery/tests/support/mise_acceptance.rs` was converted and compile-checked (`cargo check -p kuru-delivery --all-targets`, lint) but not run
