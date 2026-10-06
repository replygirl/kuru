## ADDED Requirements

### Requirement: Owner-internal exact candidate reconciliation

The managed owner SHALL offer typed candidate reconciliation only through an authenticated held candidate capability, binding a new request identity to the exact branch, expected candidate head and caller-captured live head. It SHALL settle earlier uncertain work and serialize the short operation under existing owner mutation ownership. It MUST require a clean open current-schema candidate and exact current live equality to the supplied head before effects; live movement SHALL return a definite no-effect result. An accepted operation SHALL return unchanged, committed reconciliation or explicit conflict. Every accepted mutating path MUST retain exact request-bound outcome evidence; a lost reply MUST be queried without resending reconciliation. Same-generation unknown or differently bound request identities MUST remain unproved, and restart recovery MUST first prove original owner reap. Recovery SHALL validate the exact committed candidate head, parents and effective live base, issue a fresh checked handle for an open ref and preserve ambiguity. A proven noncommit MUST NOT fabricate the lost original conflict/no-op result. Existing candidate creation, promotion, abandonment and selected-ref outcomes MUST retain their exact identities. This is an internal step of checked dream publication, not a public stale-base merge command or forced promotion.

#### Scenario: Checked capability reconciles unrelated progress
- **WHEN** an authenticated candidate holder reconciles an exact clean head while ordinary live rows advanced
- **THEN** only that candidate receives the checked merge, main stays unchanged, and the response identifies the exact target and effective promotion base

#### Scenario: Foreign or changed request is refused
- **WHEN** a caller supplies a foreign handle, stale generation, changed head, non-open ref or historical schema
- **THEN** reconciliation refuses before mutation without adopting another candidate or changing live memory

#### Scenario: Exact outcome survives transport loss
- **WHEN** an accepted reconciliation reply is lost in the same generation or after owner restart
- **THEN** the original request is inspected without replay, only exact evidence clears uncertainty, and missing or unproved evidence cannot be reported as a committed merge

#### Scenario: Existing selected resolution uses updated inspection
- **WHEN** a reconciled candidate remains open for explicit resolution
- **THEN** its inspected effective base/head and fresh authenticated handle agree, and only a new exact selected abandonment request may dispose of it
