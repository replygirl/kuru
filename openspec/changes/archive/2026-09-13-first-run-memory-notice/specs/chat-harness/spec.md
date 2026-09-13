## MODIFIED Requirements

### Requirement: Conventional interactive and scripted chat

The application SHALL provide a terminal chat interface with model, effort and mode controls, status, visible parts and relationships, input, cancellation, and a scripted run command with JSON output. After actual writable project memory is available, a pending first-run memory notice SHALL be written and flushed to stderr before headless runtime work, or shown as a system transcript entry only after the first successful completed TUI draw and before input admission. The notice MUST use the escaped actual managed directory and list the notes, export, selected-forget, and purge-help controls. It MUST remain outside model prompts, peer histories, `TurnOutput`, and machine stdout. Help, inspection, configuration, authentication, models, tools, sessions, and missing-store paths MUST NOT open memory merely to show or record it.

#### Scenario: Offline first run
- **WHEN** a user starts a demo conversation without provider credentials
- **THEN** a complete response is produced with inspectable participating peers.

#### Scenario: Pending interactive notice
- **WHEN** a first interactive writable project reaches its completed initial frame
- **THEN** the frame shows the informational memory controls before input and records the version only after that draw succeeds.

#### Scenario: Headless JSON notice
- **WHEN** a pending writable project runs a headless runtime command with JSON output
- **THEN** stdout remains parseable JSON while the notice is flushed on stderr before work and is not repeated after reopening.

#### Scenario: Provider-free undo notice
- **WHEN** a pending writable project runs `undo-dream`
- **THEN** it follows the same stderr-before-work and durable-version boundary without starting a provider.
