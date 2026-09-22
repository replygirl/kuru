## ADDED Requirements

### Requirement: MCP catalog metadata is not tool authority

Kuru SHALL filter MCP tools by their original server names before provider exposure and SHALL create executable routes only from a current successful discovery of an enabled alias. A provider-visible cached entry MUST be identified as stale and MUST fail execution without dispatch. Every live MCP call MUST continue through the existing stable selector and permission evaluator after route resolution.

#### Scenario: Provider proposes a stale cached tool
- **WHEN** a provider proposes a tool that is present only in a valid stale catalog
- **THEN** Kuru returns a bounded unavailable-route result without contacting the server, consuming a grant, or treating cache metadata as an execution decision

#### Scenario: Filtered live tool is invoked
- **WHEN** a permitted original MCP tool survives current allow and deny filtering and a provider proposes its projected name
- **THEN** Kuru resolves the stable alias and original tool name, evaluates the ordinary permission decision, and dispatches through the current live session exactly once

#### Scenario: Disabled or denied stale tool is proposed
- **WHEN** a previously cached tool is disabled or denied by the current configuration
- **THEN** Kuru neither exposes nor routes it even if its old metadata remains on disk

