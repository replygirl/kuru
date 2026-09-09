## Why

The first visual pass established a promising welcome screen, but repeated labels,
disconnected controls and one shared graph still make the full interface feel
unfinished. Interactive selections also fail to survive a fresh launch, so the
visual promise of persistent parts does not match the everyday experience.

## What Changes

- Develop a quieter, more distinctive composition with a unified composer dock
  pairing model, effort and framework values with their controls.
- Give all four frameworks distinct terminal-native scenes, continuous restrained
  ambient motion and typing feedback while preserving accurate actor activity.
- Remove redundant panel labels and the prominent motion control; motion stays on
  by default with the existing startup accessibility override available.
- Complete selection flows with discoverable previews, clear feedback, responsive
  layouts and durable per-project mode and provider-specific model/effort choices.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `chat-harness`: complete expressive controls, framework identity, ambient motion
  and durable project preferences.

## Impact

Terminal rendering/input, CLI startup and runtime preference APIs change. Preferences
use the existing SQLite store outside tool roots; no new dependency, protocol or
credential handling is needed. Launch overrides remain transient. Existing sessions
remain readable and new launches still start fresh conversations unless resumed.

## Surfaces

- [x] interactive — terminal composition, controls, feedback and restart behavior
- [ ] deploy — deployment topology
- [ ] integration — external contracts
- [ ] agent-behavior — prompts, tools, routing or output shape
