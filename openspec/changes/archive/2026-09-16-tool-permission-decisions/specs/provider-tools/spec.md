## ADDED Requirements

### Requirement: Deterministic effect-bound tool decisions

Kuru SHALL evaluate bounded permission rules against the actual validated invocation at `ToolHost::execute` and before runtime outbound `a2a_send`. Selectors SHALL identify native tool names, MCP alias plus original tool name, or configured outbound A2A alias rather than provider-facing hashed names or descriptions. Optional file patterns SHALL match normalized, anchored project-relative targets only; absolute or traversing patterns MUST be rejected. Any matching deny SHALL win over matching ask, which SHALL win over matching allow, independent of rule order. With no matching rule, ordinary read/list tools and configured MCP/A2A calls SHALL retain their post-trust allow behavior; legacy `allow_write` and `allow_shell` true SHALL allow their respective calls, and false SHALL require approval. Ask-capable tools SHALL remain discoverable. Neither discovery nor a direct caller MAY bypass execution-time evaluation, the workspace root, protected paths, or trust preflight.

#### Scenario: Deny wins over grants and rule order
- **WHEN** an invocation matches deny, ask and allow rules in any order and a prior grant exists
- **THEN** Kuru returns a typed denied result and performs no external effect.

#### Scenario: Legacy false requests approval
- **WHEN** a validated write or shell call has no matching explicit rule and its legacy boolean is false
- **THEN** an attached foreground caller may request approval, while an unattended caller receives a typed permission-required refusal before dispatch.

#### Scenario: Direct and routed tools share the gate
- **WHEN** the same native, MCP or outbound A2A invocation is submitted through direct CLI or a model tool call
- **THEN** its effective rule and grant decision is the same; configured MCP startup still requires its separate workspace trust preflight.

#### Scenario: File matching cannot widen authority
- **WHEN** a file operation targets a checked path containing glob metacharacters or a path outside the checked workspace
- **THEN** exact-file grants treat the metacharacters literally, and neither a pattern nor a grant can authorize an outside or protected path.

### Requirement: Checked and revocable tool grants

An approval request SHALL bind the immutable tool identity, validated target, full arguments, retained workspace identity and effective authority context, while its bounded redacted display SHALL be separate from that authorization identity. Kuru SHALL revalidate the retained root immediately before dispatch. Once approval SHALL cover only that exact invocation; session approval SHALL cover the displayed exact-file or whole-tool scope only for the running session; always approval SHALL persist that scope in checked private Kuru state, bound to native root identity, complete reviewed manifest and effective permission/tool-route context. A matching explicit deny MUST remain effective despite any grant. Grant-store failure, cancellation, a closed approval surface, or an unanswered request MUST NOT authorize dispatch. The connector service and runtime outbound A2A path SHALL return a typed permission-required denial when no attached foreground surface can resolve an ask. Session and always grants SHALL be revocable; uncertain dispatch MUST NOT preserve a once grant for replay.

#### Scenario: Changed invocation cannot consume once approval
- **WHEN** a once approval was issued for one exact invocation and arguments, target, workspace identity or route change before execution
- **THEN** the changed operation receives a fresh decision and the old approval produces no effect.

#### Scenario: Persistent grant loses authority context
- **WHEN** a saved always grant is reopened after its reviewed manifest or effective MCP/A2A route changes
- **THEN** it no longer authorizes a call, and no raw command, argument or credential is needed in the grant record.

#### Scenario: No foreground decision is available
- **WHEN** an ask decision occurs in direct CLI, unattended A2A, dreaming or a closed/cancelled foreground interaction
- **THEN** it settles promptly as a typed permission-required refusal with no tool, process, file or network effect.
