# Proposal

## Why

Exact main c2786319's Windows ARM behavior partition 3 fails `busy_resume_and_continue_refuse_before_cli_provider_catalog` after its awaited local driver and exchange closes, at the immediate empty live-driver assertion. The local close acknowledgment confirms socket disposal; the independent owner releases the exact connection claim after observing EOF, so a query on another connection can precede that settlement.

## What Changes

Before launching either competing resume, observe the owner's empty live-driver inventory under the existing service-operation test budget, yielding between checked queries. Retain the separate final empty assertion and all existing busy-resume/continue refusal, unchanged revision, zero losing catalog/inference, one winning driver and exact public-settlement assertions. A failure to settle remains an error; no elapsed wait is treated as completion.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Connection-bound claim release and immediate zero-client retirement requirements remain unchanged.

## Impact

Only `apps/kuru-tui/tests/cli.rs` plus this fix's required records. No production API, memory/session lifecycle, protocol, provider policy, tool pins, dependencies, timeouts or coverage changes. Original failure logs are before-fix evidence; native Windows execution remains subsequent CI evidence.

## Surfaces

- [ ] interactive — no application/UI behavior change
- [ ] deploy — no workflow or execution-topology change
- [ ] integration — no external transport contract change
- [ ] agent-behavior — no prompt or routing change
