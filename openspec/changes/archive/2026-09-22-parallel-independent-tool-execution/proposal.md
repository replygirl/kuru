## Why

One provider response can contain several independent native reads, but the speaking loop currently awaits every tool call in sequence. This delays useful results and prevents activity from showing the actual concurrent work. The Phase 2 roadmap calls for parallel independent tool calls while serializing mutation; concurrency must preserve the existing permission, nested-instruction, cancellation and receipt boundaries.

## What Changes

- Classify and admit calls in provider order before effect. For `grep` and `glob`, discover bounded candidates, authorize each exact candidate and review its applicable nested instructions during this serial phase. Freeze admitted arguments, target sets and authority. A fresh foreground or Once approval, new instruction activation, or other call that cannot be safely held across admission becomes a serial barrier and executes through its existing immediate-dispatch path; later calls replan when authority changes.
- Execute contiguous admitted independent native `file_read`, `file_list`, `grep`, `glob` and `web_fetch` calls in bounded waves. Recheck each call's target, workspace, permission and instruction authority at execution; changed facts refuse or replan before exposing results. `web_fetch` retains its independent connection-time destination and redirect checks.
- Emit start and settlement activity for the actual active calls as they finish, while returning protocol-complete tool results in the provider's original call order and preserving each call ID, turn identity and budget accounting.
- Keep mutating files, cognition, shell, MCP, skill activation and unclassified calls as serial barriers. Cancellation drains the bounded wave before the turn can continue or close; an uncertain effect is never replayed.

## Capabilities

### New Capabilities

- `parallel-tool-execution`: deterministic admission, scheduling, cancellation and ordered result behavior for independent read-only tool waves.

### Modified Capabilities

- `provider-tools`: classify proven independent native reads and preserve every per-call authority check through ordered admission and parallel checked execution.
- `typed-runtime-events`: report the real active tool set and out-of-order settlement without exposing arguments or losing the provider-ordered result contract.

## Impact

The runtime speaking scheduler, connector ToolHost admission/execution boundary, tool activity projection and their native/PTY fixtures change. No storage schema, provider wire format, command syntax or release dependency is added. The work builds on the archived verified-file-edit checkpoint contract; its mutating effects remain serial.

## Surfaces

- [x] interactive — TUI activity reflects concurrent calls and their settlement.
- [ ] deploy — no deployment topology changes.
- [ ] integration — no new third-party or wire contract.
- [x] agent-behavior — a provider tool batch can overlap only independently admitted reads.
