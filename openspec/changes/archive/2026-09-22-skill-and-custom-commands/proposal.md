## Why

Kuru currently loads project instructions eagerly but has no progressive skill catalog or explicit custom prompt commands. Users cannot select a reusable `SKILL.md` procedure without manually pasting it, and a slash name outside the built-in registry is rejected even when the user has authored a project-specific prompt.

## What Changes

- Discover bounded Agent Skills metadata from user and project skill directories, disclose only the selected skill body and selected reference on demand, and give actors an explicit `skill_load` selection tool.
- Capture project skill metadata, selected body/reference bytes, and project custom-command bytes with checked source provenance; review effective project prompt authority through the existing exact-root once/persist/deny workspace trust path.
- Register user and project custom prompt commands alongside built-ins with deterministic collision rules; expand an invoked command into an ordinary user turn without shell execution.
- Document source paths, precedence, supported metadata fields, bounds, trust behavior, and the distinction between a referenced document and process/tool authority.

## Capabilities

### New Capabilities

- `skill-discovery`: bounded metadata catalog and on-demand body/reference activation.
- `custom-commands`: explicit user and project prompt-command discovery and invocation.

### Modified Capabilities

- `workspace-trust`: source-bound project prompt claims and complete supplemental approval for selected skill material.
- `command-registry`: dynamic prompt entries share help, completion, and dispatch with built-ins.

## Impact

Core snapshot/manifest capture, connector skill selection, runtime prompt continuation, TUI command registry/dispatch, user documentation and curated docs change. The trust store keeps its existing exact-root record and review UX; no storage migration or new general extension runtime is planned. A pinned YAML metadata parser may be added if required for standard frontmatter syntax, with Cargo.lock updated in the same change.

## Surfaces

- [x] interactive — skill and command discovery, help, completion and foreground review
- [ ] deploy — no deployment topology change
- [x] integration — Agent Skills `SKILL.md` frontmatter and checked reference layout
- [x] agent-behavior — metadata prompt, selected instructions and prompt-command turns
