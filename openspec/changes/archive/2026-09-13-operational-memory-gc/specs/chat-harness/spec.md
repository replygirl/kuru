## ADDED Requirements

### Requirement: Dream candidate resolution

Dream failure or cancellation SHALL settle accepted candidate writes before it
explicitly abandons the candidate, and abandonment cleanup SHALL continue in an
owned worker if its caller stops waiting. Process loss or abnormal termination
without a completed explicit transition MUST leave the ordinary candidate
recoverable. A confirmed fast-forward SHALL remain the authoritative promoted
outcome even if later candidate-ref cleanup is incomplete, so cleanup failure
MUST NOT suppress live runtime publication.

#### Scenario: Settled dream is abandoned

- **WHEN** a dream fails or observes cancellation after creating its candidate
- **THEN** accepted writes settle before explicit abandonment, exact resolved-ref cleanup is attempted without replay, and an unconfirmed result remains recoverable.

#### Scenario: Promotion wins cleanup failure

- **WHEN** the candidate fast-forward is confirmed but candidate-ref cleanup fails or loses its reply
- **THEN** the runtime publishes the promoted topology and memory exactly once while retained cleanup state remains safe to retry.

#### Scenario: Process stops without explicit resolution

- **WHEN** a process stops while an ordinary candidate has no durable promotion or abandonment transition
- **THEN** the candidate branch and its private history remain intact and startup does not delete or promote it.
