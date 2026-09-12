## Why

Memory configuration accepts relative cache and development-binary paths even
though the memory lifecycle needs a stable native location. The core validator
does not express that policy, and direct memory entry points can begin creating
private directories before rejecting a malformed configuration.

## What Changes

- Make `MemoryConfig::validate` the single policy boundary for a one-to-300
  second startup timeout and optional nonempty, NUL-free, native absolute
  `cache_dir` and `dolt_binary` paths.
- Validate configuration at public memory opening and provisioning entry points
  before they create any store or cache directories.
- Document that those two memory overrides are absolute paths; keep CLI
  `--data-dir` normalization and relative configured tool paths unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing versioned-memory capability already owns managed runtime
configuration; this repair makes its existing private-location contract fail
closed at every public memory entry point.

## Impact

`kuru-core` configuration validation, `kuru-memory` open and provision
boundaries, their isolated regression tests, and the two configuration
references. No memory schema, retention policy, raw `OpenOptions::data_dir`
contract, server timeout guard, runtime behavior, or TUI behavior changes.

## Surfaces

- [ ] interactive — completed conversation and operation feedback in the TUI
- [ ] deploy — deploy/runtime/CI-execution topology
- [ ] integration — a third-party/external contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
