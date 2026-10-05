# Spec Delta

## ADDED Requirements

### Requirement: TUI configuration inspection is a working read-only command

The interactive command registry SHALL expose `/config` with no arguments in help, leading-token completion, parsing, and local dispatch. The command SHALL display the invocation's captured effective configuration with final-leaf provenance, applying the already-loaded saved project preferences without rereading configuration files or repeating workspace review. It SHALL show current live mode, model, and effort selections separately from the captured configuration and label them as current session state. Saved preference provenance SHALL identify `saved project preferences` only when that preference actually wins. The display SHALL redact secret-bearing configuration values, including MCP environment values, static MCP header values, URL user information, and sensitive URL query values, while retaining configured environment-reference names without resolving them. Bounded output SHALL visibly report omitted or truncated entries. `/config` SHALL be local and read-only: it MUST NOT issue a provider request, change configuration or project preferences, invoke a new authority review, open an additional memory store, or perform a memory mutation.

#### Scenario: Discover and inspect captured configuration
- **WHEN** a user requests help, completes `/con`, and submits `/config` in an idle TUI
- **THEN** help, completion, and dispatch agree on `/config`, and the TUI shows captured effective values with the source of each final leaf plus a separate current-session selection section, without a provider request

#### Scenario: Saved and live selections remain truthful
- **WHEN** a saved project preference wins during startup, then a user successfully changes a model, mode, or effort in the TUI and invokes `/config`
- **THEN** the captured projection identifies the winning saved preference and the separate live section shows the successfully selected current value; a refused selection leaves the displayed live value unchanged

#### Scenario: Inspection uses the captured snapshot
- **WHEN** a configuration source changes after the TUI has entered its reviewed session and the user invokes `/config`
- **THEN** the command shows only the already captured effective snapshot and does not reread the changed source or prompt for workspace trust

#### Scenario: Secret-bearing values are redacted
- **WHEN** configuration contains sentinel values in MCP environment entries, static header values, URL user information, or sensitive query parameters, and names for environment references
- **THEN** the displayed configuration contains none of the sentinel values, retains those environment-reference names, and reports safe source provenance

#### Scenario: Oversized inspection is visibly bounded
- **WHEN** effective configuration exceeds the display row or text bound
- **THEN** the TUI renders bounded output and a visible notice identifying that entries or values were omitted or truncated

#### Scenario: Invalid command arguments stay local
- **WHEN** a user submits `/config unexpected`
- **THEN** the TUI explains that `/config` takes no arguments without dispatching a provider request or changing session state
