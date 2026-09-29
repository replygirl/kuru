# Proposal

## Why

The Windows arm64 engine input job restores a pin-verified archive keyed on
eight whole files, including `Cargo.lock` and the memory package's
`mise.toml`/`mise.lock`. Only main saves the cache, so every pull request that
edits one of those files rebuilds the engine from source on every run (about
7m24s instead of about 7s), ahead of every windows-11-arm partition. Most of
those edits cannot change the engine bytes.

## What Changes

- `.github/workflows/bundle-build.yml`: the `windows-arm64-input` cache key
  keeps the target and the committed pin, and replaces the eight-file hash with
  the inputs that decide the archive bytes: the asset manifest, the helper's
  `main.rs`, `bundle.rs` and `bundle/build.rs`, the ZIP writer
  (`packages/kuru-archive/src/zip.rs`), and the `Cargo.lock` records of the ZIP
  writer's locked crate closure (named in a job `env` list and digested by a new
  `run` step that fails when a named crate is missing). Test modules, the
  memory package's consumer build script, `mise.toml` and `mise.lock` leave the
  key. Restore verification, the rebuild path, the build pin check and the
  main-only save are unchanged.
- `packages/kuru-delivery/tests/bundle_build.rs`: a test derives the build's
  inputs from the helper's CLI default manifest, its module declarations, the
  modules its keyed files use and `Cargo.lock`. It requires the key to cover
  exactly those inputs and the determinism job's `pull_request.paths` to include
  them. Because `mise.toml` and `mise.lock` leave the key, the test also fails
  closed on any change to the `bundle:build` and `setup:build-tools` tasks, the
  mise configuration the task loads, the input job's environment and steps, and
  the compression crates' Cargo features. Named exemptions stay outside both the
  key and the test, as on main, and the pin check backs them. The toolchain test
  derives its tools from the task instead of a fixed pair.
- `docs/development.md`: the engine input cache, its key, what the test
  enforces and what only the pin check covers.

## Impact

Only the `Pin-verified Windows arm64 engine input` job, which native-tests and
release call. Its first run under the new key misses once on main and saves.
No secrets, job names or required checks change; `ci-gate` and `Lint PR title`
stay the required checks.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
