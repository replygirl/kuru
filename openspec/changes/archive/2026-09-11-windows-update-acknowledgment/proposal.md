## Why

Native Windows CI 34602482369 published the updated executable successfully after 11.7 seconds, but the parent had already closed its acknowledgment pipe after ten seconds. The startup frame deadline incorrectly includes verification, copying the bundled executable twice and durable publication, so a valid update reports failure and requires recovery.

## What Changes

Give verified publication its own bounded acknowledgment wait, separate from connection startup and frame transfer. Preserve receipt validation, exact image acknowledgment, helper ownership, failure reconciliation and short malformed-frame limits. Exercise a real helper whose observed publication work exceeds the startup wait, then require installed offline update acceptance on Windows.

## Capabilities

### Modified Capabilities

None: the existing native update and recovery contracts remain correct.

## Impact

The Windows update protocol in packages/kuru-delivery, its native helper fixture and acceptance records. No authentication, peer runtime, dependency, public configuration or release topology changes.

## Surfaces

- [x] interactive — successful native updates return their verified installation result
- [ ] deploy — existing helper process and native CI topology retained
- [ ] integration — no external protocol changes
- [ ] agent-behavior — no agent behavior or evaluation changes
