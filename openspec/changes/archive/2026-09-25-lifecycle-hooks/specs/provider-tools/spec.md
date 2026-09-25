## ADDED Requirements

### Requirement: Hooked tools retain final-operation authority and settled results

For each admitted provider tool call, Kuru SHALL preserve its original call identity while applying any configured pre-tool chain before dispatch and any post-tool chain after immutable settlement. The final rewritten name and arguments MUST be decoded and rerun through the existing schema, budget, containment, permission, and execution checks. A pre-hook denial or failure MUST settle the call without executing it. A post-hook denial, failure, or annotation MUST leave the actual tool result, effect, receipt, usage, and settled observation unchanged. Raw hook command details, stdout, stderr, and unrelated credentials MUST NOT enter provider-visible tool results.

For dream `dream_suggest` calls, the authored proposal cap MUST be enforced before hook admission. A rewritten call MUST pass the dream-only tool name and proposal validators; it MUST NOT gain ordinary file, shell, MCP, or external-agent authority. Its receipt and any post annotation MUST remain in the same candidate actor view, with abandonment removing both from live memory.

#### Scenario: Rewrite changes tool class

- **WHEN** a pre-tool hook rewrites a permitted read proposal into a shell or mutation proposal without its required grant
- **THEN** Kuru refuses the final operation before process or filesystem effect and settles the original call ID once

#### Scenario: Post annotation accompanies exact result

- **WHEN** a post-tool hook returns a valid visible annotation after a successful read
- **THEN** the provider receives the original result and separately identified fitted annotation under the same call identity, without a second tool effect
