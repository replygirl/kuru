## Why

The pre-push coverage run intermittently lost two native connector peers: an
expected Codex error assertion failed and a JSON-RPC peer closed its output.
The immutable executable fixture currently discovers its plan through
`current_exe`, making test identity depend on platform executable-path behavior.
The failure cause is not yet proven; startup and assertion diagnostics are too
sparse to distinguish a missing plan from other premature process exits.

## What Changes

Make the absolute invocation path the explicit identity of each fixture plan,
preserving immutable shared executable bytes and independent transcripts.
Exercise that contract with a real subprocess whose executable and invocation
paths differ, and improve failure evidence without changing protocol assertions.
Investigate any further observed failures rather than adding retries or weakening
the repository gate.

## Capabilities

### Modified Capabilities

None; this corrects test infrastructure, not a product contract.

## Impact

Connector test support, native stdio fixture and test failure diagnostics only.
No production provider, process supervision, dependencies or release semantics
change. Follow-up evidence belongs to this cospec record.

## Surfaces

- [ ] interactive — no product interface change
- [x] deploy — native fixture execution in local and hosted repository gates
- [ ] integration — no external protocol change
- [ ] agent-behavior — no prompt or routing change
