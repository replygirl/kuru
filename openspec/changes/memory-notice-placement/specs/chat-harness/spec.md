# Spec Delta

## MODIFIED Requirements

### Requirement: Conventional interactive and scripted chat

The application SHALL provide a terminal chat interface with model, effort and mode controls, status, visible parts and relationships, input, cancellation, and a scripted run command with JSON output. After actual writable project memory is available, a pending first-run memory notice SHALL be written and flushed to stderr before headless runtime work, or shown as a status-line indicator in the first successful completed TUI draw and recorded only after that draw and before input admission. The interactive indicator MUST NOT be a transcript entry, MUST NOT expire on the transient-notice timer, and SHALL clear on the first key or paste. Both forms MUST use the escaped actual managed directory, state that memory is kept until removed, and point to `kuru memory --help`, whose long help lists the notes, export, selected-forget, and purge-help controls. The interactive `/status` output SHALL always include the escaped managed memory directory. The notice MUST remain outside model prompts, peer histories, `TurnOutput`, and machine stdout. Help, inspection, configuration, authentication, models, tools, sessions, and missing-store paths MUST NOT open memory merely to show or record it.

#### Scenario: Offline first run
- **WHEN** a user starts a demo conversation without provider credentials
- **THEN** a complete response is produced with inspectable participating peers.

#### Scenario: Pending interactive notice
- **WHEN** a first interactive writable project reaches its completed initial frame
- **THEN** the status line shows the memory indicator with the directory before input, the transcript does not contain it, and the version is recorded only after that draw succeeds.

#### Scenario: Indicator clears on input and stays out of the conversation
- **WHEN** the user types after the indicator was shown and later completes turns
- **THEN** the indicator is gone from the status line, never reappears in the transcript, is not stored as a message, and `/status` still names the memory directory.

#### Scenario: Headless JSON notice
- **WHEN** a pending writable project runs a headless runtime command with JSON output
- **THEN** stdout remains parseable JSON while the notice is flushed on stderr before work and is not repeated after reopening.

#### Scenario: Provider-free undo notice
- **WHEN** a pending writable project runs `undo-dream`
- **THEN** it follows the same stderr-before-work and durable-version boundary without starting a provider.
