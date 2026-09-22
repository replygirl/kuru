# custom-commands Specification

## Purpose
Define how checked project and user prompt commands enter the terminal registry, how collisions resolve, and how invocation preserves ordinary turn and tool-permission boundaries.

## Requirements

### Requirement: Explicit prompt-command sources and precedence

Kuru SHALL discover only documented user and project custom-command Markdown entries with valid bounded metadata and nonempty prompt bodies. It SHALL capture project command bytes and source identities in the base immutable trust manifest before showing their names, help or completions. Built-in command names SHALL win all collisions; effective project commands SHALL win equal-named user commands. Shadowed or invalid project entries MUST NOT add effective prompt authority. Discovery and omissions SHALL be deterministic and bounded.

#### Scenario: Built-in and project collision
- **WHEN** a project command file names an existing built-in command
- **THEN** the built-in remains the sole help, completion and dispatch entry and the project file never becomes executable command authority.

#### Scenario: Project and user collision
- **WHEN** valid user and project prompt files share a custom name
- **THEN** the approved project entry supplies help, completion and dispatch for that name, while the user entry is shadowed.

### Requirement: A custom command starts an ordinary user turn

Invoking a registered custom command SHALL expand its immutable captured prompt body and optional literal arguments into an ordinary user turn in the current session. Kuru MUST NOT interpret the entry as shell syntax, execute a process, infer a tool grant, or accept arbitrary unregistered slash strings as custom commands.

#### Scenario: Prompt entry with arguments
- **WHEN** a user invokes a registered `/review path` prompt command
- **THEN** the next provider request sees the captured review prompt and a separately labeled literal `path` argument in the same session, with no process launch.

#### Scenario: Unknown slash string
- **WHEN** a user submits a slash name absent from the effective registry
- **THEN** Kuru reports an unknown command without starting a provider turn.
