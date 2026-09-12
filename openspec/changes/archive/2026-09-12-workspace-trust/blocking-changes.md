# Dependencies

## Blocked by

- [x] `memory-config-validation` — supplies the single validated memory-path policy consumed while deriving a side-effect-free authority snapshot *(archived 2026-09-12)*

## Soft-blocked by

None.

## Siblings

Archived `authoritative-turn-display` and `provider-response-budgets` changes
touched adjacent TUI and connector source files but provide no runtime
prerequisite for preflight. Coordinate future source integration without
treating either as a trust gate.

## Existing foundations

Archived native-platform, Dolt-memory, OpenAI-authentication, and
memory-config-validation changes provide the checked directory, private-state,
validated memory-path, and fixed ChatGPT route foundations that this feature
composes. The former memory validation prerequisite is resolved; no unarchived
hard prerequisite remains.
