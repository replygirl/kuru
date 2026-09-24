## MODIFIED Requirements

### Requirement: Private summary isolation and bounded display projection

Reasoning-summary records SHALL remain producer-private and session-scoped.
During an admitted selected-speaker turn, Kuru SHALL preserve the existing
bounded transient preview of streaming reasoning-summary text. That preview
MUST use only the text fragment, MUST discard provider coordinates, MUST clear
when the active turn ends, and MUST NOT create or read a durable
reasoning-summary record. Settled producer-private payloads and their provider
coordinates MUST NOT enter assistant transcripts, peer prompts, session export,
forks, cross-session continuity, or durable public runtime event detail.

#### Scenario: active selected-speaker preview remains transient
- **WHEN** the selected speaker receives streaming reasoning-summary text
  during an active admitted turn
- **THEN** the interactive preview receives only the existing bounded text tail
- **AND** it contains no provider coordinates and clears with that turn

#### Scenario: display projection does not disclose the private payload
- **WHEN** an interactive surface requests the visible summary state for an
  active selected-speaker turn
- **THEN** it receives only the existing bounded transient text tail authorized
  for that surface
- **AND** the persisted producer-private payload and raw provider coordinates
  are absent from durable public event and transcript paths

#### Scenario: settled private records remain out of public paths
- **WHEN** a reasoning-summary sidecar is accepted after a successful terminal
  completion
- **THEN** its durable record and provider coordinates are absent from
  transcripts, peer prompts, session export, forks, cross-session continuity,
  and durable public runtime event detail

#### Scenario: session export and fork exclude producer-private summaries
- **WHEN** a session containing reasoning-summary records is exported for
  presentation or forked
- **THEN** neither operation contains or imports those records into the target
  transcript, context, or continuity state
- **AND** the invoking owner's separate full-memory export continues to retain
  the private records for backup and restore
