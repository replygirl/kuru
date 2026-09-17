## ADDED Requirements

### Requirement: Foreground tool approval and permission review

The attached interactive terminal SHALL offer bounded, redacted inline once/session/always/deny choices for an ask decision, showing the exact scope that a session or always grant would cover. The default file grant scope SHALL be the validated exact project-relative target; shell, MCP and outbound A2A SHALL show their whole-tool identity. The terminal SHALL expose current permission state through a dock chip and `/permissions` inspection with session/always revocation. A prompt MUST NOT hold a memory transaction or UI lock, and cancellation, quit, a closed approval channel or a newer operation MUST fence a late decision. Headless/scripted output SHALL remain final-only and shall not present an interactive approval prompt.

#### Scenario: Exact-file approval and revocation
- **WHEN** a user approves an asked file operation for this session and later revokes its displayed exact-file scope through `/permissions`
- **THEN** the first checked operation executes, another path receives a new decision, and the revoked scope authorizes no later call.

#### Scenario: Persistent whole-tool approval
- **WHEN** a user approves a displayed MCP, shell or outbound A2A whole-tool scope as always and restarts under the same checked workspace/authority context
- **THEN** the checked grant remains visible and usable until revoked, without rendering raw arguments or secrets.

#### Scenario: Cancellation or denied prompt
- **WHEN** the user denies, cancels or closes an inline request before a decision is applied
- **THEN** no effect is dispatched, the composer remains usable, and a late approval cannot authorize a newer operation.
