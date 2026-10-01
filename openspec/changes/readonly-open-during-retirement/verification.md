# Verification

## 1. A read-only inspection after owner retirement reads its own generation [critical]

- [ ] 1.1 @regression (agent) `managed_inspection_meeting_a_retiring_owner_waits_for_its_reap_and_reads_its_own_generation` against a real owner paused after endpoint retire and after reap -> not yet run; must pass with a managed read-only open and fail for a borrowing open
- [ ] 1.2 @integration (agent) `mise run //apps/kuru-tui:test` preferences tests with the converted fixture -> not yet run; both pass
- [ ] 1.3 @runtime (agent) repeated local runs of `an_explicit_framework_override_is_validated_against_its_own_part_budget` -> not yet run; all pass (the unforced local baseline was 20 of 20 before the fix, so this alone does not prove the fix)

## 1b. A reset met while connecting to a retiring owner is "no owner" [critical]

- [ ] 1.4 @regression (agent) `maintenance_connect_queued_before_the_listener_dropped_waits_for_the_owner_lock` -> not yet run; `request_idle_retirement` returns `None`, then the maintenance permit waits for the owner lock and acquires after the reap. Expected: passes on macOS before the fix (connect Ok, handshake EPIPE); the ubuntu leg exercises the connect-ECONNRESET arm and is expected, by inference, to fail without the fix
- [ ] 1.5 @regression (agent) `a_client_connect_queued_before_the_listener_dropped_is_a_peer_closed_miss` -> not yet run; `try_attach_observed` gives `AttachMiss::PeerClosed`
- [ ] 1.6 @unit (agent) routing unit test: `ConnectionReset` in the maintenance connect context is `is_peer_closed` and not `is_transport_unavailable` -> not yet run
- [ ] 1.7 @unit (agent) a live owner that keeps resetting while holding its lock is reported at the existing deadline with the peer-closed count named -> not yet run
- [~] 1.8 @runtime (agent) native ubuntu and Windows legs of 1.4 and 1.5 -> defer: no Linux or Windows run was made locally; the Linux mechanism is inferred from tokio and kernel source, the only observed occurrence is CI job 110146577721

## 2. Contract and same-class sites

- [ ] 2.1 @unit (agent) lint and docs checks over the corrected facade comment and converted fixtures -> not yet run
- [ ] 2.2 @manual (human) review that `terminal.rs` live-owner borrows are unchanged -> not yet run

## 3. Platforms

- [~] 3.1 @runtime (agent) native Windows and ubuntu coverage runs -> defer: no Windows evidence was collected locally; CI is the only authority for ubuntu coverage timing
