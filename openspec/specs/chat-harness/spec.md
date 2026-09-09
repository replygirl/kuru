# chat-harness Specification

## Purpose
Provide conventional terminal and scripted chat interfaces to the peer runtime,
with discoverable model, effort and framework selection, persistent sessions,
layered configuration and responsive cancellation.

## Requirements

### Requirement: Conventional interactive and scripted chat

The application SHALL provide a terminal chat interface with model, effort and mode controls, status, visible parts and relationships, input, cancellation, and a scripted run command with JSON output.

#### Scenario: Offline first run
- **WHEN** a user starts a demo conversation without provider credentials
- **THEN** a complete response is produced with inspectable participating peers.

### Requirement: Layered configuration and project instructions

The application SHALL merge user, ancestor project and explicit local configuration in that order, reject unknown or invalid options, and load ancestor AGENTS.md instructions with clear local precedence.

#### Scenario: Local override
- **WHEN** a project config changes the user default mode and explicit local config changes it again
- **THEN** the local mode is effective while unrelated inherited options remain.

### Requirement: Durable session continuity

The application SHALL persist session state and histories outside tool roots and expose session inspection and resumption.

#### Scenario: Restart
- **WHEN** a process exits and a new process resumes the same session
- **THEN** its conversation and peer topology remain available.
