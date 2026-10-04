# Spec Delta

## ADDED Requirements

### Requirement: Canceled pooled-operation retirement

An abnormally dropped pooled memory session SHALL close its owned connection through a bounded disposal path without waiting for the canceled operation's pending SQL response. The connection SHALL retain its client pool permit until disposal completes or its existing close bound expires. Successful explicit release SHALL continue to return the connection inline for reuse. Candidate retirement SHALL retain its admission fence, existing caller deadline and exact server-session absence proof before renaming or deleting a branch; local pool counts alone MUST NOT authorize that transition. Uncertain writes SHALL retain their existing exact-session reconciliation fence.

#### Scenario: Return task already started after query cancellation

- **WHEN** a transaction is canceled after a real query executes but before its response reaches the client, and disposal begins while the pool remains open
- **THEN** pool retirement can finish without delivering the held query response, and branch transition still waits for exact server-session absence

#### Scenario: Successful pooled statement releases inline

- **WHEN** a successful statement or explicit transaction completion releases a session
- **THEN** the next statement can reuse the same idle connection before another connection is opened
