## MODIFIED Requirements

### Requirement: Explicit shared-store shutdown

`MemoryStore::close` SHALL consume its calling view and await release of that view's client attachment. Dropping a view SHALL release only that view; after an explicit close, retained clones of that attachment SHALL reject reads and writes with the same clear closed-store error. Closing or dropping one attachment SHALL NOT stop a shared service or invalidate another client's attachment. The service owner SHALL await shutdown of its pools and owned supervisor after the last client and accepted operation have drained and the idle interval expires, or after an authenticated maintenance request finds no other attached client. A read-only attachment SHALL not gain authority over an external owner through close or drop.

#### Scenario: A clone is dropped
- **WHEN** a caller drops one writable clone while retaining another
- **THEN** the retained clone can continue to read and write its live store

#### Scenario: A view is explicitly closed
- **WHEN** a caller explicitly closes one view while retaining a clone
- **THEN** reads and writes through the retained clone fail with `memory store is closed`, while an independent attachment remains usable

#### Scenario: Final client closes
- **WHEN** the final client closes after its accepted operations settle
- **THEN** the service waits its idle interval unless authorized maintenance requests early retirement, and must reap Dolt before releasing the lifecycle lease
