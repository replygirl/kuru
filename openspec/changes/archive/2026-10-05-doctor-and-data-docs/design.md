# Design

## Context

The CLI already has typed subcommands, immutable configuration snapshots, workspace trust inspection, native authentication status, and a verified embedded-engine asset manifest. Memory ownership is managed separately by `kuru-memory`; a doctor command must observe that state without entering its normal writable open or service-start path. The current public documentation describes memory and installation in separate pages but has no consolidated doctor/troubleshooting/privacy/threat-model flow.

## Operational surface

This change adds a local CLI command and curated static documentation only. It
opens no listener, adds no container or hosted service, requires no new secret,
and does not change supported binary versions or architectures. Optional
inspection of an existing memory owner uses a bounded local connection to the
already-published project service; it adds no bind address or connection limit.

## Goals / Non-Goals

**Goals:**
- Compose existing safe local observations into one stable report.
- Make memory inspection explicitly no-start and bounded.
- Keep Responses route discovery subordinate to existing workspace trust.
- Publish concise operational guidance grounded in current code.

**Non-Goals:**
- Proving whole-binary authenticity, cold SQL health without an existing owner, or general system readiness.
- Repair, migration, service election, engine extraction, network activity, credential refresh, or shell execution.
- Reading personal credentials or activating repository-selected configuration authority.

## Decisions

### Keep diagnosis in the parsed CLI path

Add an explicit `Doctor` Clap command and dispatch it from the typed command enum. Do not scan raw arguments for a command name: parsed command dispatch handles aliases and option ordering correctly and prevents accidental activation from an unrelated argument.

### Report bounded observations with fixed values

Represent results with finite status/reason enums and serialize only those values. Do not copy account metadata, source paths, config values, or remote error text into a report. Configuration parse failures and uncertain file I/O remain distinct local observations.

### Inspect memory without starting it

Split structural validation from live database inspection. Structural checks validate canonical project/activation metadata without opening writable state. A live query is permitted only through the already-published compatible owner using a read-only handle and a finite timeout; all other cases stop with an explicit unverified result. Do not reuse normal open/attach-or-start flows because they can provision state or alter service lifetime.

### Preserve route and trust boundaries

Use the fixed ChatGPT credential status independently. Responses key presence is observed only after its configured environment-variable name is authorized by the applicable trust manifest; no value is included in output. Never fall back between the two routes.

### Document exact limits rather than a readiness score

Pages describe the implemented observations and their gaps, including non-expiring memory, target-dependent compressed engine size, no secure-erasure promise, and shell process authority. A composite readiness/authenticity field was rejected because it would imply guarantees this command cannot establish.

## Risks / Trade-offs

- [Risk] A read-only attach can still wait on an unavailable owner → Use the existing finite handshake bound and report unknown/unverified on timeout.
- [Risk] Local status may be mistaken for provider connectivity or full database integrity → Use explicit labels and document that no provider request or cold SQL health check occurs.
- [Risk] Repository config can name an environment variable before approval → Resolve the name only after the existing trust decision; otherwise omit the Responses environment check and report it unverified.
