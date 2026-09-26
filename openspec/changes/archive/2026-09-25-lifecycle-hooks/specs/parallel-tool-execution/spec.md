## ADDED Requirements

### Requirement: Lifecycle hooks preserve parallel-call ordering

P14 SHALL run each call's pre-tool chain inside P06's original-order admission before that call can dispatch. Independent calls MAY retain P06 concurrency after their own admission. Each call's post-tool chain SHALL begin only after that call has one immutable settlement and MAY complete independently, but provider continuation MUST retain original call order, unchanged call IDs, exact settled results, and per-call ordered annotations. A hook failure or rewrite that creates a replan boundary MUST stop later dispatch according to the existing ordered-admission contract rather than allowing later calls to execute under stale authority.

#### Scenario: Reverse settlement with annotations

- **WHEN** two admitted independent reads settle and finish their post-hook chains in reverse order
- **THEN** activity may reflect actual settlement timing while provider continuation contains both unchanged results and their annotations in original call order

#### Scenario: Earlier rewrite activates new authority

- **WHEN** an earlier pre-tool rewrite changes instruction or permission authority relevant to a later proposed call
- **THEN** the later call does not dispatch under its stale proposal and the existing replan semantics remain authoritative
