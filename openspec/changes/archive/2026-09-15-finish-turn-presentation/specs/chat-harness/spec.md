## ADDED Requirements

### Requirement: Reachable exact retry and durable interruption presentation

The scripted `run` command SHALL accept an optional bounded caller turn ID, and
the TUI SHALL offer `/retry` only for its single last durably retained local
submission. A new local submission MUST atomically retain its exact ID, prompt
and raw target with the started journal and sole user transcript row; retry MUST
reuse that tuple without replacing it. A completed retry MUST perform no new
provider/tool work and MUST NOT duplicate the displayed or durable user/answer
pair. A possibly dispatched incomplete turn MUST refuse retry without side
effects. A2A message-ID idempotency SHALL remain separate and MUST NOT replace
the local TUI reference.

When an admitted turn settles without a completed output, one fixed nonsecret
interruption marker MUST be committed through the journal/transcript path and
remain visible after later turns and resume. The marker MUST NOT claim that
external work was absent or rolled back and MUST be excluded from provider
conversation context. Repeated settlement MUST NOT duplicate it. If the ended
answer checkpoint wins a race, the authoritative answer SHALL be shown and no
interruption marker added.

#### Scenario: Completed last-turn retry
- **WHEN** `/retry` or `run --turn-id` exactly repeats a completed local submission in its original session
- **THEN** the stored output is reused with zero provider/tool calls and no duplicate transcript or TUI pair.

#### Scenario: Unsafe last-turn retry
- **WHEN** the retained turn may have dispatched external work
- **THEN** `/retry` refuses without calls and explains that a new submission is new work.

#### Scenario: Safe pre-dispatch retry
- **WHEN** a retained turn was interrupted before possible dispatch
- **THEN** `/retry` may complete that same logical turn without another user row, retaining the earlier interruption marker.

#### Scenario: Persistent interruption
- **WHEN** an interruption settles and later work completes or the session is resumed
- **THEN** exactly one Kuru interruption marker remains in the transcript without entering provider context.

#### Scenario: Completion wins cancellation
- **WHEN** the ended checkpoint commits before cancellation is reconciled
- **THEN** the committed answer is returned and displayed without a false interruption marker.
