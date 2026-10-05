# Tasks

## 1. Versioned state storage

- [x] 1.1 Add scalar and coherent bounded batch versioned reads alongside historical value-only batch reads, and validated atomic conditional writes in the owning module; real-Dolt race, stale rollback, absent and retry fixtures verify the contract.
- [x] 1.2 Add schema 9 and version advancement for every actual state overwrite; migration, old-registry refusal, checkpoint/candidate and historical fixtures verify preservation and invalidation.

## 2. Private service contract

- [x] 2.1 Route operations and typed stale faults through facade, exhaustive receipt contract and protocol bump/pin; contract fixtures and real managed stale/recovery probes verify no fence or receipt regression.
- [x] 2.2 Verify exact accepted-write recovery and concurrent all-or-nothing behavior across Local and managed Remote stores using real bundled Dolt and event synchronization.

## 3. Documentation and acceptance

- [x] 3.1 Document row versions, schema compatibility, unchanged export and remaining topology/admission boundary in owning docs; docs build/content/link checks verify accuracy.
- [x] 3.2 Run package-owned focused and owning memory checks, static host/Windows-target checks, formatting and strict/managed Cospec checks; record observed results and identify hosted acceptance separately.
