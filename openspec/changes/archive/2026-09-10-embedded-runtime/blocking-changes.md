# Dependencies

## Blocked by

- [x] `dolt-memory` — verified full-Dolt catalog, provisioning and lifecycle being embedded *(archived 2026-09-10)*

## Soft-blocked by

None.

## Review and sequencing

Reviewed all active proposals (dolt-memory, native-windows, embedded-runtime) and
the archived delivery/build/install/release providers on 2026-09-10. The user's
explicit Dolt, bundling and portability instructions authorize this sequence:
Dolt foundation, embedded runtime, then native Windows. Native Windows consumes
this contract; it is not a prerequisite for embedding existing native targets.
No further permission or publication action is implied.
