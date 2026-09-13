## ADDED Requirements

### Requirement: MCP degradation reference

The curated documentation SHALL explain that MCP tools are discovered per configured server, failed servers are reported without hiding built-in or healthy-server tools, failed or ambiguously cancelled calls are not replayed, and recovery occurs only through a later explicit discovery. It MUST explain that bounded stdio stderr is recognizable-secret projected and terminal escaped for human diagnostics only, while runtime status is fixed metadata, and MUST retain the documented finite-detector, process-authority, and no-sandbox limitations.

#### Scenario: User diagnoses one failed server

- **WHEN** a configured MCP server fails while another server remains healthy
- **THEN** the tools and protocol references explain which tools remain available, where the safe status and optional stderr diagnostic appear, and why Kuru neither replays the failed call nor claims arbitrary-secret or escaped-process containment
