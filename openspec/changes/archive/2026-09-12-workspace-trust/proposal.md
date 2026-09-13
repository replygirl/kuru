## Why

Automatically discovered ancestor configuration can currently select process,
mutation, executable, credential-routing, and outbound-agent authority before
the user has reviewed or trusted that workspace. Configuration also needs a
single retained view: approval of one set of files cannot safely authorize a
later reload with different authority.

## What Changes

- Add a side-effect-free, immutable, provenance-aware configuration snapshot
  that captures every invocation override before review, derives a redacted
  manifest for effective authority contributed by automatic ancestor
  configuration, and reports malformed configuration without raw values.
- Gate activation at the exact canonical `-C` root with private, identity- and
  manifest-bound approval state; provide status, approval, revocation, a
  one-invocation grant, and a pre-alternate-screen interactive choice.
- Apply the approved command activation matrix before memory, provider,
  credentials, `ToolHost`, MCP or external-agent activation; retain the
  approved root through `Harness` and configured-spawn construction; preserve
  ordinary settings and explicit caller input without prompting.
- Revalidate the retained workspace before pathname-based shell and stdio-MCP
  launches, report the Unix cwd-binding limitation honestly, and document that
  trust does not sandbox a same-user process.

## Capabilities

### New Capabilities

- `workspace-trust`: provenance-aware configuration authority manifests,
  exact-workspace approvals, and activation preflight.

### Modified Capabilities

- `chat-harness`: layered configuration becomes a one-shot snapshot whose
  automatic-ancestor authority is preflighted before runtime activation.
- `provider-tools`: provider credential routes, tools, MCP and A2A activation
  observe workspace preflight without changing shell's process-authority claim.
- `public-documentation`: user documentation describes workspace trust,
  command behavior, invalidation and its explicit limits.

## Impact

`kuru-core` gains snapshot/provenance and manifest types; `kuru-platform` may
expose a narrow retained-directory revalidation primitive; the TUI owns trust
storage, commands, preflight and authentication ordering; connectors revalidate
before configured cwd-based spawns. `kuru-runtime` accepts a retained tool host
and exposes provider-free compensating dream undo while preserving live-harness
publication and recovery. User docs and native fixture coverage change.
BREAKING: unapproved automatic ancestor authority now fails closed instead of
activating during startup. No Git-root discovery, OS sandbox, handle-bound Unix
cwd launch, Phase 1 permission language, memory schema, or history-retention
policy is included.

## Surfaces

- [x] interactive — terminal trust commands and pre-TUI approval choices
- [ ] deploy — deploy/runtime/CI-execution topology
- [x] integration — provider, MCP and A2A activation contracts
- [x] agent-behavior — configured tool and external-agent exposure
