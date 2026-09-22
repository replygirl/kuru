## ADDED Requirements

### Requirement: Layered managed and local invocation behavior

Kuru SHALL resolve configuration in the order built-in defaults, managed ordinary defaults, existing user file, outer-to-inner ancestor project files, saved mode/model/effort preferences, discovered project-local file, explicit `--config` file, repeatable `-c` values, and dedicated CLI flags. It SHALL enforce managed constraints on memory and other preflight values before activation, then check saved mode/model/effort against their locks after reading those preferences and before provider, tool or prompt dispatch. `kuru config` SHALL inspect the resolved values without opening memory and SHALL redact MCP environment values from every layer.

#### Scenario: Precedence across local and CLI
- **WHEN** a user file, project file, untracked project-local file and `-c` each select a different mode
- **THEN** the `-c` mode is effective and an explicit dedicated `--mode` value supersedes it.

#### Scenario: Constraint after saved preference
- **WHEN** a remembered model choice or a CLI override conflicts with managed policy
- **THEN** configuration fails before constructing a configured provider or reading its credentials.
