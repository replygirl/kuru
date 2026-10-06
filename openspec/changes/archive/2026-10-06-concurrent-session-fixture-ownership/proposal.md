# Proposal

## Why

PR243's native coverage runs execute 26 distinct failing fixtures after session-scoped driver admission lands. Existing fixtures write private session state through unclaimed attachments, observe an input view rather than the independently bound runtime view, or omit the checked recovery probe's connection lifecycle; these setups no longer reach their original mutation/recovery assertions. Ordinary runtime CLI opens also bypass the former checked private-data and lock-directory preflight, losing actionable privacy errors. The local lease regression further proves memory initialization happens before native session admission checks maintenance exclusion; its original legacy/no-effect assertions detect that side effect. Both production edges can be corrected without a global exclusive conversation lease.

## What Changes

- Restore checked private-data and lock-directory preflight before writable runtime memory activation, retaining session-scoped admission and existing error mapping. Hold the existing checked project lock in shared mode through startup/admission and memory cleanup so exclusive maintenance refuses before initialization, while independent ordinary sessions coexist. Legacy activation retains its exclusive lease until complete, then reacquires shared startup ownership before further admission effects.
- Bind and select actual exact-session driver claims for managed fixture writes, including migrated legacy sessions, and retain/close ownership through paused replies and recovery.
- Use the runtime's bound view for shared driver work, service reply barriers and dream lease refusal observation; independently claimed older sessions retain separate private authority.
- Assert the complete checked-recovery probe, identified successor connection and connection release sequence. After local presence close, observe the owner's actual empty live inventory within the fixture's existing deadline before asserting release.
- Keep original exact-once, committed-before-reply-loss, migration durability, privacy, cursor, foreign-driver refusal, EOF retirement and provider-count assertions unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing ownership, recovery and private-directory requirements remain correct.

## Impact

Production: `apps/kuru-tui/src/cli.rs` privacy preflight and shared maintenance exclusion before runtime activation. Fixtures: app lease/terminal tests; memory facade, driver and migration tests; runtime engine, accounting, dream and provider-free undo tests. No schema, protocol, dependencies, tool pins, deadlines or driver enforcement changes. The already committed unconditional Arc import and immutable archives remain intact.

## Surfaces

- [x] interactive — actionable CLI privacy/refusal diagnostics and existing terminal fixtures
- [ ] deploy — no workflow or execution-topology change
- [ ] integration — no provider or wire contract change
- [ ] agent-behavior — no prompt, routing or output policy change
