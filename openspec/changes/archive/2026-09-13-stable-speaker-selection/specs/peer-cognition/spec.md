## ADDED Requirements

### Requirement: Stable speaker selection

The runtime SHALL preserve existing caller-target and active-focus precedence.
For automatic selection it SHALL choose only among successful deliberating peers,
prefer the greatest reported activation, retain the previous completed speaker
when that peer shares the greatest activation, and otherwise choose the first
identity in stable ascending order. Turn count MUST NOT change this tie-break.
The last completed speaker SHALL persist with session state and old sessions
without that value SHALL remain readable. Selection reasons SHALL be visible in
the event trace without changing existing speaker-event meaning or peer authority.

#### Scenario: Equal activations across turns and resume
- **WHEN** the same eligible peers share the highest activation across completed turns or after resuming the session
- **THEN** the previous completed speaker remains selected independently of turn count.

#### Scenario: A different peer has greater activation
- **WHEN** an eligible peer has strictly greater activation than the previous speaker
- **THEN** the higher-activation peer is selected and the reason identifies activation as decisive.

#### Scenario: Missing or ineligible prior speaker
- **WHEN** there is no previous completed speaker or that identity is not among the eligible maximum-activation peers
- **THEN** the stable identity tie-break selects an eligible maximum without creating or reviving an actor.

#### Scenario: Turn fails before completion publication
- **WHEN** speaking fails or is cancelled before publishing completed session state
- **THEN** it does not replace the session's last completed speaker.

#### Scenario: Existing target and focus behavior
- **WHEN** an existing caller target or valid active focus selects the identity
- **THEN** that precedence remains effective and its reason is observable without requiring a new command or setting.
