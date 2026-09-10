# Dependencies

## Blocked by

- [x] `peer-harness` — persistent peers and existing memory contracts *(archived 2026-09-09)*
- [x] `native-build-tooling` — package-owned Rust delivery tooling *(archived 2026-09-09)*
- [x] `direct-release-install` — compiler-free installation baseline *(archived 2026-09-09)*

## Soft-blocked by

None.

## Authorization and scan

At creation, this was the only active change. The independent test-only
`stable-stdio-fixtures` change now repairs an existing native fixture race found
by the full gate; it does not change storage or protocol behavior.
The user previously placed Dolt after the
initial release; v0.1.0 and the installer change have shipped. The user now
explicitly requests replacing SQLite with Dolt, satisfying that sequencing gate.
No further release or external service provisioning is required for this change.
