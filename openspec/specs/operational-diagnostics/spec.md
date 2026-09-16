# operational-diagnostics Specification

## Purpose
Defines bounded, redacted operational diagnostics for runtime-owning CLI commands.

## Requirements

### Requirement: Bounded project diagnostics

The application SHALL initialize one checked owner-private, fixed-count and fixed-byte JSONL diagnostic ring only for a runtime-owning command after its project writer lease is held and before memory opens. It MUST retain that ring through harness and memory cleanup, finish it before releasing the writer lease, and report bounded setup or write failure without changing an already committed turn result. Read-only and non-runtime commands MUST NOT create the ring.

#### Scenario: Runtime command
- **WHEN** an approved runtime-owning CLI command starts for a canonical project
- **THEN** it records only bounded structured operational fields in that project's existing ring and finishes the ring before writer-lease release.

#### Scenario: Read-only command
- **WHEN** configuration, auth, models, notes, sessions, tools, or selected-note forgetting runs
- **THEN** it creates no diagnostic ring.

### Requirement: Redacted asynchronous turn observations

Runtime and connector code SHALL emit subscriber-neutral asynchronous spans/events for turns, actor work, external tool execution, and typed retry decisions. Records MUST use the existing domain-separated journal digest for turn correlation and MAY contain only stable identifiers, literal operation/status categories, attempt, elapsed time, and already-available byte or token counts. They MUST NOT contain prompts, histories, tool arguments/results, configuration, endpoints, headers, raw caller turn IDs, or error chains.

#### Scenario: Retry and tool result
- **WHEN** a fake provider retries and actor work invokes a successful or failing tool
- **THEN** correlated records contain typed retry and tool status observations without their fake secret inputs or outputs.

#### Scenario: Cancellation
- **WHEN** an asynchronous turn is cancelled
- **THEN** its turn and child observations close without a long-lived entered span guard across awaits.

### Requirement: Fixed diagnostic presentation

The global `--debug` flag SHALL add only documented operational detail to the
application's fixed target allowlist and SHALL print the resolved checked
project ring directory once on stderr after installation. It MUST NOT enable
`RUST_LOG`, arbitrary third-party logs, raw payloads, or unsolicited stdout/TUI
output. Documentation MUST identify the four 64 KiB `trace-{0..3}.jsonl` files
as a bounded operational ring distinct from the durable semantic turn journal.

#### Scenario: Machine output
- **WHEN** `kuru run --json --debug` starts diagnostics
- **THEN** stdout remains the exact parseable command result and stderr names the existing checked ring directory without exposing a credential.

#### Scenario: Debug and journal distinction
- **WHEN** a user inspects storage documentation
- **THEN** the operational ring is described as fixed-count and bounded while turn journals are described as no-expiry and proportional to turns.
