## MODIFIED Requirements

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect
instruction/configuration paths. Mutations and shell execution MUST require
explicit opt-ins and matching workspace approval when automatic ancestor
configuration contributes their effective grant. Shell execution MUST have time
and independent stdout and stderr retained-output bounds and SHALL be described
as process authority rather than a filesystem sandbox; workspace approval does
not widen tool roots or make private same-user state inaccessible to a shell.
Built-in file-read and shell output that exceeds its retained-output budget MUST
remain a bounded visible head-and-tail excerpt after recognized-secret projection,
rather than fail solely for crossing that retention budget. The excerpt MUST keep
UTF-8 boundaries and every visible recognized-secret marker whole. Model-facing
tool receipts MUST retain the call ID and valid JSON while applying the same
bounded excerpt rule. MCP stdio, HTTP JSON, and SSE framing/parser admission
bounds remain separate protocol limits.

#### Scenario: Symlink escape
- **WHEN** a filesystem call follows a workspace symlink outside the root
- **THEN** the operation fails without modifying the outside file.

#### Scenario: Pending shell or write grant
- **WHEN** an automatic ancestor enables shell or writes without matching approval
- **THEN** the tool host neither exposes nor executes that authority.

#### Scenario: Oversized built-in shell streams
- **WHEN** shell stdout or stderr exceeds its independent retained-output budget
- **THEN** the completed tool result preserves a marked head and tail for that
  stream, drains both streams through EOF within the existing operation and
  cleanup authority, and does not combine their budgets or extend its deadline.

#### Scenario: Tail survives a model receipt boundary
- **WHEN** a projected built-in tool result exceeds the model receipt budget
- **THEN** the receipt remains valid JSON with its original call ID and contains
  a marked head-and-tail excerpt without a partial recognized-secret marker.
