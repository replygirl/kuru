# Spec Delta

## ADDED Requirements

### Requirement: Typed conditional state service operations

The private storage service SHALL expose scalar versioned, value-only batch and coherent versioned batch reads and
conditional state publication through the existing generation-bound facade.
Conditional writes MUST be classified explicitly as logical receipt-bearing
mutations. A stale response SHALL retain bounded key/expected/actual metadata as
a definite refusal distinct from uncertain transport or storage failure. The
protocol minor and exact pin SHALL change together; no session driver admission
or arbitrary SQL authority SHALL follow from these operations.

#### Scenario: Stale managed write
- **WHEN** an attached client publishes with a stale expectation
- **THEN** it receives the typed stale refusal and may issue a fresh mutation
  without an unresolved accepted-write fence

#### Scenario: Uncertain managed write
- **WHEN** a reply is lost after a conditional request was accepted
- **THEN** the facade fences subsequent writes until the exact logical unit
  outcome is reconciled, including after checked successor attachment
