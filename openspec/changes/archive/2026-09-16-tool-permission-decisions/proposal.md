## Why

Kuru currently uses write and shell booleans as hard execution switches, so a default interactive session cannot request permission for a useful operation at the moment it is needed. Tool proposals also cross several execution paths; a consistent decision must be enforced at each external-effect boundary without weakening workspace trust or the checked tool root.

## What Changes

- Add bounded allow/ask/deny rules for stable native, MCP and outbound A2A selectors, with optional anchored project-relative file patterns and deny-first precedence. BREAKING: legacy `allow_write = false` and `allow_shell = false` become `ask` rather than unconditional refusal; `true` remains allow, and an explicit matching deny always wins.
- Evaluate immutable invocations at `ToolHost::execute` and before runtime `a2a_send`, with typed permission-required denials and no effect before an allow or valid grant. Keep workspace trust as the prior gate for automatic configuration and MCP startup.
- Add a foreground-only, cancellation-aware approval service and checked private once/session/always grants with exact displayed scope, authority-context binding and revocation. Headless callers receive a typed refusal instead of waiting.
- Present inline once/session/always/deny choices, a permission chip and `/permissions` inspection/revocation, and publish matching strict configuration schema and documentation.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-tools`: call-site permission evaluation for native tools, MCP and outbound A2A; typed denials and checked grants.
- `workspace-trust`: automatic permission rules join the reviewed authority manifest without becoming tool grants.
- `configuration-schema`: strict `permissions` rule shape and parser/schema parity.
- `chat-harness`: foreground approval, visible scope and permission inspection/revocation.

## Impact

Core config and pure matcher, published schema, workspace trust manifest, connector ToolHost permission service, runtime outbound A2A path, app-owned private grant storage, TUI controls and existing headless refusal paths change. No database migration, new tool/protocol/config layer, standalone CLI grant-management command, OS sandbox, control server, usage ledger or mode policy is introduced. Existing shell, file-root, protected-path, MCP preflight and native process-lifetime checks remain authoritative.

## Surfaces

- [x] interactive — inline decisions, permission state and revocation
- [ ] deploy — no deployment topology change
- [x] integration — configured MCP/A2A tool routes and published config schema
- [x] agent-behavior — advertised requestable tools and enforced tool execution
