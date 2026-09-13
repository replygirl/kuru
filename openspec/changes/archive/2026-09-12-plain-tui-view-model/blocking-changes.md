# Dependencies

## Blocked by

- [x] `authoritative-turn-display` — typed completed-turn delivery, completion metadata, and generation-checked cancellation behavior that this refactor preserves *(archived 2026-09-12)*

## Soft-blocked by

None.

## Independent active lanes

- `bounded-provider-diagnostics` changes connector diagnostics and public
  documentation, not the TUI view-model seam.
- `verify-dolt-schema-transaction-boundary` changes only the memory package's
  recovery-test fixtures and does not supply this refactor.
