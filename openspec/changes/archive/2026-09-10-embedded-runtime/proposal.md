## Why

Installing Kuru must supply everything needed to run its memory engine. The Dolt
foundation currently downloads its pinned engine at first use, which leaves an
otherwise successful install dependent on another network operation and fails a
fresh offline start. The user explicitly requires portable, bundled dependencies.

## What Changes

- Embed the verified target-specific full Dolt archive, including its licenses,
  in every Kuru executable; extract it privately without runtime downloads.
- Prepare immutable pinned build inputs through a memory-owned mise task and
  a native delivery helper; Cargo validates local bytes without networking.
- Carry the complete engine through direct, mise, source and native update paths.
- Exercise packaged offline conversations and persistent memory, and document
  the build-input and end-user installation contracts in the owning guides.

## Capabilities

### New Capabilities

- `embedded-runtime`: complete executable distribution, verified offline runtime
  extraction and explicit reproducible build preparation.

### Modified Capabilities

- `versioned-memory`: replace the existing-cache prerequisite with verified
  bundled cold startup, including offline first use. The companion native-windows
  change extends the platform set after this contract is established on the
  existing four native release targets.

## Impact

Touches kuru-memory provisioning/catalog/build script and mise tasks, the native
kuru-delivery helper, source install and build entrypoints, native workflow smoke
tests, documentation and AGENTS.md. No memory schema or peer behavior changes.
Removes the runtime network provisioning path. An explicit exact-version engine
override can remain for development, but ordinary builds always contain Dolt.

## Surfaces

- [x] interactive — first-use CLI/TUI errors and offline startup
- [x] deploy — binary payload, build inputs, installation and update tests
- [x] integration — pinned upstream archive and license validation
- [ ] agent-behavior — no prompt, routing or output changes
