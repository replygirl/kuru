## ADDED Requirements

### Requirement: Provider-delivered private summary admission

Kuru SHALL persist a `reasoning_summary.v1` record only when a successfully
settled provider completion delivers a typed reasoning-summary item. The record
MUST retain the admitted session, controlled-turn, speaking actor, provider
invocation, final settled item, output, and summary coordinates supplied by the
runtime. It MUST NOT derive a summary from transcript text, opaque provider
continuation, a Harness-lifetime operation identifier, or another session's
history.

#### Scenario: settled provider item is retained with its real turn identity
- **WHEN** a provider completion settles a reasoning-summary item for an
  admitted controlled turn
- **THEN** the private record carries that turn's session, actor, invocation,
  final-item, output, and summary coordinates
- **AND** retrying the same settled item does not create another record

#### Scenario: no provider summary is delivered
- **WHEN** a completion has no typed reasoning-summary item or fails before a
  successful terminal completion
- **THEN** Kuru persists no reasoning summary and does not synthesize one from
  other response material

### Requirement: Private summary isolation and bounded display projection

Reasoning-summary records SHALL remain producer-private and session-scoped.
Kuru MAY derive a bounded display projection from an admitted record, but the
private payload and its provider coordinates MUST NOT enter assistant
transcripts, peer prompts, session export, forks, cross-session continuity, or
public runtime event detail.

#### Scenario: display projection does not disclose the private payload
- **WHEN** an interactive surface requests the visible summary state for a
  settled turn
- **THEN** it receives only the bounded display projection authorized for that
  surface
- **AND** the persisted producer-private payload and raw provider coordinates
  are absent from the public event and transcript paths

#### Scenario: session export and fork exclude producer-private summaries
- **WHEN** a session containing reasoning-summary records is exported for
  presentation or forked
- **THEN** neither operation contains or imports those records into the target
  transcript, context, or continuity state
- **AND** the invoking owner's separate full-memory export continues to retain
  the private records for backup and restore

### Requirement: Exact session and item isolation

Private summary lookup SHALL require the exact persisted session and admitted
identity tuple. A record from one actor, invocation, turn, or final item MUST
NOT satisfy another tuple, including when visible text or provider item indexes
coincide.

#### Scenario: equal-looking summaries remain distinct
- **WHEN** two sessions or two provider invocations deliver the same visible
  reasoning-summary text
- **THEN** each record remains reachable only through its own admitted identity
  tuple and neither is used as continuity for the other
