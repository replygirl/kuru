## ADDED Requirements

### Requirement: Reviewed automatic permission authority

Every effective permission rule supplied by automatic ancestor configuration SHALL be included with final-leaf provenance in the immutable exact-root authority manifest before its affected tool or protocol authority activates. Approval of that workspace manifest SHALL authorize use of the reviewed configuration, but SHALL NOT itself create a tool grant or override a matching permission deny. Invocation-only trust SHALL remain scoped to that command's applicable claims, and changing the reviewed manifest SHALL invalidate always grants bound to the former authority context.

#### Scenario: Repo rule cannot activate before review
- **WHEN** an automatic ancestor adds an allow or ask rule affecting a tool invocation without matching workspace trust
- **THEN** Kuru may show a bounded redacted claim but does not construct or execute that configured authority before applicable review.

#### Scenario: Trust is not tool approval
- **WHEN** a workspace manifest is approved but an invocation's effective decision is ask or deny
- **THEN** ask still needs a valid foreground/grant decision and deny still performs no effect.
