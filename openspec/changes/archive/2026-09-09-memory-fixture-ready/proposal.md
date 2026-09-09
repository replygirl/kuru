## Why

The memory process fixture waits for a readiness line that equals `ready`. With one libtest thread, the runner prefixes that line with the test name, so the parent never releases its children and the check hangs.

## What Changes

Recognize a distinct readiness token independently of libtest's line prefix. Always run the single selected child test with one harness thread, so ordinary runs exercise the formerly broken output format while the four child processes still initialize together.

## Capabilities

### Modified Capabilities

None. Production behavior and the concurrency requirement are unchanged.

## Impact

Only packages/kuru-core/tests/memory.rs changes. No production code, dependency, global test serialization or CI timeout changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration — Rust libtest child-process output
- [ ] agent-behavior
