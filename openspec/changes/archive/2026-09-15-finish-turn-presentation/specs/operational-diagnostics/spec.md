## MODIFIED Requirements

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
