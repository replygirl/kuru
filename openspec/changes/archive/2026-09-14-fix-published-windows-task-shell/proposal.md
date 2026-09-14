## Why

The published Windows verification task embeds PowerShell syntax directly in `run_windows`, but mise launches that field through `cmd.exe` by default. Release `v0.3.0` verification therefore failed on the literal `$misePath` assignment before any verifier behavior ran.

## What Changes

- Route `packages/kuru-delivery/mise.toml` through an explicit package-owned `pwsh.exe` script for the Windows task.
- Preserve the verifier's exact version, commit, mise identity, run URL, evidence receipt, and environment behavior.
- Add a native Windows regression that invokes the actual mise task and proves the explicit script boundary is reached.

## Impact

Only the delivery package's published-Windows verification task, its support script, and focused workflow/task tests change. Other hosts keep the existing explicit Windows-only failure, and release assets or application behavior do not change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
