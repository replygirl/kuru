## Why

Published Windows verification for `v0.3.1` stopped before application behavior because `mise settings get url_replacements` exits nonzero when that optional setting is unset. An absent setting means there are no URL replacements and should not be treated as a command failure.

## What Changes

- Query `mise settings ls url_replacements --json` and accept an absent scoped field as an empty replacement map.
- Validate the expected JSON and map shape and continue rejecting configured HTTP or HTTPS replacements.
- Cover a real isolated mise installation with the setting absent, configured field cases, and genuine command, timeout, parse, and schema failures.

## Impact

Only the delivery-owned published-Windows verifier and focused tests change. Exact release version, commit, mise identity, receipt, and installation behavior remain unchanged; published assets are immutable.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
