## Why

The generic apps/tui directory does not identify the application it owns.
The user requests apps/kuru-tui to leave room for future branded or internal TUIs.

## What Changes

Move apps/tui to apps/kuru-tui and update workspace membership, tooling paths,
current documentation and instruction links. The Cargo package and executable
remain kuru; CLI behavior, storage, protocols and UI are invariant.

## Impact

Application source paths, Cargo workspace manifest, mise tasks and current docs.
Historical archived change records retain their original observed paths.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
