## ADDED Requirements

### Requirement: Checked independent native read admission

The ToolHost SHALL expose a narrow classification and checked admission/execution boundary for native `file_read`, `file_list`, `grep`, `glob` and `web_fetch`. An admitted read MUST bind the exact arguments, selected permission, retained native target identity and reviewed workspace/instruction context; search MUST bind only its bounded, individually allowed candidate set. Retained native capabilities MUST have one ToolHost-wide resource bound across all prepared calls; a call that cannot reserve its complete capture SHALL release partial captures and use the ordinary serial path without truncating its normal candidate set or result. A fresh foreground or request-bound Once answer and any newly activated instructions SHALL be reported as a serial boundary, never converted into a transferable approval. Checked execution SHALL preserve the existing output, omissions, redaction, destination policy and no-replay semantics of the corresponding ordinary tool call. Other tools SHALL have no parallel admission through this boundary.

#### Scenario: Read classification does not grant authority

- **WHEN** a model proposes an eligible read that is denied, requires review or changes checked target before execution
- **THEN** classification gives it no extra permission, no denied candidate is exposed, and checked execution refuses or requires the existing review path before effect

#### Scenario: Bound fetches remain isolated

- **WHEN** two already-authorized `web_fetch` calls run concurrently
- **THEN** each uses its own request, redirect and connection-time destination checks, bounded response handling and cancellation, without shared provider or MCP credentials

#### Scenario: Native identity capture exceeds the prepared-call budget

- **WHEN** one search or the accumulated prepared wave cannot retain every checked target identity within the ToolHost handle bound
- **THEN** partial captures are released and that call executes through the ordinary serial path with its existing search limits and complete bounded result
