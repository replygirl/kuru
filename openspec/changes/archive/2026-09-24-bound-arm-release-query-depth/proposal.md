## Why

The exact P29 head compiles and passes local combined coverage, but its Ubuntu ARM release build fails while compiling the `kuru` library with Rust's `queries overflow the depth limit` error. Rust reports a depth increase of 130 while computing the layout of the existing TUI shutdown async block; merged main passed the same ARM job, and the block's source is unchanged.

## What Changes

Set a finite recursion limit of 256 on the `kuru` library crate so its release build can complete the observed layout query. Do not change the TUI operation, runtime behavior, or other crates; if the bound is still insufficient, diagnose the type growth instead of repeatedly increasing it.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `apps/kuru-tui/src/lib.rs` changes. There is no public API, dependency, schema, or migration change. The exact-head Ubuntu ARM release build is the acceptance gate.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
