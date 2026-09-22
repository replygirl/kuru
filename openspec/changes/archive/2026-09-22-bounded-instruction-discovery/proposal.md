## Why

Kuru currently captures only ancestor `AGENTS.md` files. A project that uses `CLAUDE.md` and its `@path` composition convention cannot make those rules available to the actor without copying them into another file. The current aggregate cap also rejects the entire otherwise usable project when one instruction source is too large.

## What Changes

- Compose applicable `AGENTS.md` and `CLAUDE.md` sources in deterministic outermost-to-most-local order, expanding bounded relative `@path` imports once by checked source identity and detecting cycles.
- Capture exact reviewed bytes and source/directory identities in the instruction authority claim. Keep imported instructions out of prompts until their complete manifest has passed workspace trust review.
- Omit an over-cap source or import branch with an explicit bounded notice and continue with the remaining captured instructions; never present omitted bytes as active authority.

## Capabilities

### New Capabilities

- `project-instructions`: Bounded, ordered composition of ancestor and project-root instruction sources and imports.

### Modified Capabilities

- `workspace-trust`: Captured imported instruction authority participates in exact-root review before activation.
- `chat-harness`: The runtime receives only the currently applicable, reviewed instruction projection.

## Impact

`kuru-core` instruction capture and manifest types, `apps/kuru-tui` workspace review and visible omission notices, their tests, and configuration/user docs change. No configuration schema key, database migration, provider route, or new approval store is introduced. Nested path activation is a dependent, separately reviewable change; this slice adds no unused activation interface.

## Surfaces

- [x] interactive — newly captured instruction claims use the existing workspace review, and omission notices are visible
- [ ] deploy — no deployment topology changes
- [ ] integration — no third-party protocol changes
- [x] agent-behavior — reviewed project instructions change the prompt
