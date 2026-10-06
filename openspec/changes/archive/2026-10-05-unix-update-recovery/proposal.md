# Proposal

## Why

Unix replacement currently leaves no durable evidence for interrupted installation, and source update delegates execution to the checkout shell script. A running Linux image whose installed name has been replaced can also fail to launch its own memory service; D2 must close these recovery and execution gaps before completion.

## What Changes

- Add bounded receipt-driven Unix replacement and automatic recovery with checked identities, same-filesystem atomic publication, forward-only recovery after possible publication and no settled backup images.
- Build explicit source checkouts through native package-owned mise tasks, consume Cargo outputs read-only, and install them through the same transaction rather than executing checkout installation scripts.
- Recover an existing transaction before ordinary command authority activates, and teach Unix bootstrap entrypoints to preserve/refuse pending transactions.
- Keep Linux self-spawns on the actual running image after pathname replacement, throughout managed-service and lifetime-supervisor startup.
- Preserve the accepted ownership preflight and unchanged Windows receipt/helper protocol. Update notices remain a separate subsequent change.

## Capabilities

### New Capabilities

- `unix-update-recovery`: Checked Unix installation transactions, automatic recovery and native source update.

### Modified Capabilities

- `project-memory-service`: Linux self-spawn selects its own compatible running image rather than a replaced pathname.

## Impact

Own Unix delivery transaction/bootstrap tests and support scripts, narrow native self-image/process launch primitives, memory service/supervisor self-selection, CLI startup/update and public installation/development docs. No memory schema, RPC minor, dependency pin, release workflow or Windows receipt changes. No rollback/recover/check command, candidate execution for validation, Homebrew installation or generic recovery framework.

## Surfaces

- [x] interactive — updater/startup diagnostics and explicit source command behavior
- [x] deploy — installed executable publication, bootstrap and memory native self-launch
- [ ] integration — no new external protocol
- [ ] agent-behavior — no provider or actor policy changes
