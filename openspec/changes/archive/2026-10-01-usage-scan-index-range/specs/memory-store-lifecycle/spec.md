# Spec Delta

## ADDED Requirements

### Requirement: Linear usage ledger validation

Validating ledger-owned usage state at writable open SHALL read each ledger-owned row a bounded number of times, so that its cost grows at most linearly with the number of ledger-owned rows. Owned-state paging SHALL select by an exact byte range on the state key and preserve byte-wise key order.

#### Scenario: Aged ledger opens within linear cost
- **WHEN** a writable open validates a ledger holding N ledger-owned rows
- **THEN** each page reads only its own key range, and validation cost grows at most linearly with N.

#### Scenario: Range paging preserves the owned set and order
- **WHEN** the state table holds keys inside the owned prefix and keys that only resemble it (the prefix without its final separator, keys that sort immediately before or after the range, and keys with high or NUL bytes)
- **THEN** owned-state paging visits exactly the keys that begin with the owned prefix, each once, in byte-wise order.
