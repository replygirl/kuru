# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination, not a block

No active change on this branch blocks this one, and none of the overlaps below is a cospec change slug that can be ledgered here. This change stays in test code (plus the one visibility edit named in the proposal), so it does not wait on either item.

- PR #208 (`fix/instrumented-child-outlives-test`) edits `packages/kuru-memory/src/service.rs` product code in `spawn_service` (hunks at lines 1372-1441, +25 lines net). This change's lowest `service.rs` site is 1532, so the hunks do not overlap textually, but both touch the same file: whichever merges second rebases, and the line numbers in the proposal's site table shift by 25 after 1441 if #208 merges first.
- `fix/endpoint-record-replaced-name` (change `endpoint-record-replaced-name`) holds only its cospec scope commit today and no kuru-memory code. It will edit the endpoint-record read path in `service.rs` product code, a region this change does not touch.
