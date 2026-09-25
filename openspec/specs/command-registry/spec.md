# command-registry Specification

## Purpose
Define the working interactive slash-command catalog, editor completion, and local view controls so help, dispatch and visible behavior stay consistent as later command backends arrive.

## Requirements

### Requirement: Working built-in commands have one registry

The interactive TUI SHALL define every public built-in slash command through one typed registry that supplies its canonical name, usage/help entry, completion eligibility, and dispatch identity. Help, completion, and dispatch MUST agree on the registered names. A command whose backend is not implemented MUST NOT be advertised or completed as working; unknown slash names MUST receive an actionable error without becoming a provider turn. Private review and cancellation tokens MAY remain internal to their active modal flow and MUST NOT appear as public completions.

#### Scenario: Registered command is discoverable and executable
- **WHEN** a user requests help, completes a registered command prefix, and submits that command
- **THEN** the same canonical name and usage are presented and its working handler runs.

#### Scenario: Future command has no handler yet
- **WHEN** a named Phase 2 command's owning backend is not installed
- **THEN** help and completion omit it and submitting its slash name reports an unknown command without a provider request.

### Requirement: Slash completion preserves editor and modal authority

The TUI SHALL complete registered slash names only while the cursor is in the leading command token. Matching and cycling MUST be deterministic and bounded, MUST leave arguments and ordinary text intact, and MUST NOT execute a command. An active instruction or permission review and other modal inputs MUST retain priority over composer completion; outside a modal prompt, plain digits MUST remain composer input.

#### Scenario: Ambiguous prefix and arguments
- **WHEN** a user types a slash prefix shared by multiple registered commands and presses completion repeatedly, then edits or enters arguments
- **THEN** the editor cycles only matching registered names in stable order, resets the cycle on editing, and never rewrites the argument text or launches a handler.

#### Scenario: Approval prompt owns input
- **WHEN** an approval or instruction review is active and the user presses a plain choice digit or completion key
- **THEN** the existing modal choice rules apply and the slash-command editor does not intercept the choice.

### Requirement: View-only clear and current-session status

`/clear` SHALL clear only the current TUI's visible conversation projection and its row metadata; it MUST retain stored messages, project memory, session identity, usage, and the next-turn context. `/status` SHALL report the current session identifier and already available project/runtime selection and usage facts without starting a provider request, opening a new memory store, or claiming an estimate is a bill or quota. Both commands SHALL use the registry and leave the user able to continue the same session.

#### Scenario: Clear then continue
- **WHEN** a session with stored turns runs `/clear` and sends another message
- **THEN** the old rows disappear from that surface, the session and stored history remain intact, and the next turn uses the same context.

#### Scenario: Inspect current status
- **WHEN** a user runs `/status` in an idle session
- **THEN** the TUI shows its current session, project, model, effort, mode, focus and known usage state without a provider call or mutation.

### Requirement: Effective custom entries share the working registry

The invocation-local command registry SHALL combine the built-in catalog with only effective, approved custom prompt entries. Help, leading-token completion and dispatch MUST use that same registry and deterministic collision order. Modal permission/instruction review MUST retain priority over custom-command completion and execution.

#### Scenario: Custom command parity
- **WHEN** an approved custom entry is listed in help, completed from its slash prefix, and invoked
- **THEN** the same name and description identify the captured prompt command at every step.

#### Scenario: Unapproved project entry
- **WHEN** project custom-command authority has not passed exact-root preflight
- **THEN** its name, description and body do not appear in help, completion, dispatch or a provider prompt.

### Requirement: File checkpoint commands are working registry entries

The command registry SHALL expose bounded file checkpoint inspection, one selected file undo and explicit pruning only when their backing handlers are active. TUI and CLI file recovery paths SHALL describe an unresolved receipt honestly, keep file undo distinct from dream undo, and never advertise a placeholder command. A file undo attempt MUST use the same normal file authority checks as a native target mutation.

#### Scenario: Inspect and undo from a terminal
- **WHEN** a user inspects a settled checkpoint and selects its undo in a real terminal
- **THEN** the displayed ID/status/path identify that one edit and the handler reports either a verified restoration or a current-file conflict.

### Requirement: MCP tool inspection is a working registry entry

The shared TUI command registry SHALL register `/tools` only with its working MCP catalog handler. Help, completion, parsing, and dispatch MUST agree, and the handler MUST project the same filtered tools and per-alias disabled, live, stale, or degraded state as `kuru tools` without making a provider request or treating cached metadata as availability.

#### Scenario: Inspect tools in the terminal
- **WHEN** the user invokes `/tools` with live, stale, degraded, and disabled configured aliases
- **THEN** the registered handler renders their bounded filtered catalog and truthful status without starting a turn or exposing resolved header values

### Requirement: MCP authorization commands share one working registry

Kuru SHALL expose per-alias MCP login, status and logout through one typed command family used by CLI parsing and the TUI command registry. Help, leading-token completion, argument validation and dispatch MUST agree; these operations MUST run without a provider request or tool execution permission and MUST require the same reviewed configured-alias authority before contacting its resource or authorization server. Login MUST offer browser, no-browser and advertised device behavior truthfully; status and logout MUST remain bounded and redact all credential material.

#### Scenario: Login command parity
- **WHEN** a user discovers, completes and runs MCP login for a configured HTTP alias in the CLI or TUI
- **THEN** both surfaces select the same alias-bound connector operation and present the same flow choices without starting a model turn

#### Scenario: Unknown or ineligible alias
- **WHEN** a user requests MCP status, login or logout for an unknown, disabled, STDIO or unapproved automatically configured alias
- **THEN** Kuru refuses before discovery, credential access or process startup with an actionable bounded diagnostic
