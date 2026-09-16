## MODIFIED Requirements

### Requirement: Stable speaker selection

The runtime SHALL preserve existing caller-target and active-focus precedence.
For automatic selection it SHALL choose only among successful deliberating
peers, prefer the greatest reported activation, and retain the previous
completed speaker when that peer shares the greatest activation. When a cold
tie remains, it SHALL choose the first eligible identity in the built-in mode's
authored part order: Self first for IFS, Connection for polyvagal, Desire for
Freudian and Continuity for Jungian, then each mode's next authored identity.
Only when no authored identity is an eligible tied candidate SHALL stable
ascending ID order decide. Turn count MUST NOT change these tie-breaks. The last
completed speaker SHALL persist with session state and old sessions without that
value SHALL remain readable. Selection reasons SHALL identify the actual rule in
the event trace without changing existing speaker-event meaning or peer
authority.

#### Scenario: Equal activations across turns and resume
- **WHEN** the same eligible peers share the highest activation across completed turns or after resuming the session
- **THEN** the previous completed speaker remains selected independently of turn count.

#### Scenario: A different peer has greater activation
- **WHEN** an eligible peer has strictly greater activation than the previous speaker
- **THEN** the higher-activation peer is selected and the reason identifies activation as decisive.

#### Scenario: Cold authored facing
- **WHEN** a fresh session has multiple eligible maximum-activation peers and no previous completed speaker
- **THEN** the first eligible identity in that mode's authored order speaks and the reason is `mode-authored-order`.

#### Scenario: Missing or ineligible prior speaker
- **WHEN** there is no previous completed speaker or that identity is not among the eligible maximum-activation peers
- **THEN** authored mode order selects an eligible maximum before stable ID order is considered.

#### Scenario: No eligible authored identity
- **WHEN** a cold maximum-activation tie contains no eligible built-in authored identity
- **THEN** stable ascending ID order selects one tied identity and the reason is `stable-id-order`.

#### Scenario: Turn fails before completion publication
- **WHEN** speaking fails or is cancelled before publishing completed session state
- **THEN** it does not replace the session's last completed speaker.

#### Scenario: Existing target and focus behavior
- **WHEN** an existing caller target or valid active focus selects the identity
- **THEN** that precedence remains effective and its reason is observable without requiring a new command or setting.
