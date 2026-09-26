## ADDED Requirements

### Requirement: Hook authority joins exact-root review

Every effective lifecycle-hook command, argument, event, order, and execution bound supplied by automatic repository configuration SHALL be captured with final-leaf provenance in the immutable exact-root workspace authority manifest. Hook processes MUST NOT start and hook payloads MUST NOT be disclosed until applicable workspace approval or a command-scoped one-invocation grant succeeds. Local and managed hook configuration SHALL retain their existing provenance and constraint semantics; no hook result MAY widen the reviewed manifest, tool roots, permission rules, provider route, or mode policy. A changed retained workspace or changed effective hook authority MUST fail before the next hook launch.

#### Scenario: Changed repository hook command

- **WHEN** an approved repository changes a hook executable, argument, event assignment, order, or bound
- **THEN** the previous approval is stale and Kuru starts no hook, provider, or tool effect until the new applicable authority is reviewed

#### Scenario: Hook output requests more authority

- **WHEN** an approved pre-tool hook rewrites a call outside the existing tool root or into an explicit permission deny
- **THEN** final validation rejects the operation without treating hook approval as a tool grant
