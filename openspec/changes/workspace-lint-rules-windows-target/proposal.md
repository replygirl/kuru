# Proposal

## Why

Two gaps let a known flake class and Windows-only defects reach `main`
unchecked. First, PR #126's capture of the retained-stage event used a
thread-scoped `tracing::subscriber::set_default`. tracing-core 0.1.36 caches
callsite interest process-wide, so another test's thread cached `never` for the
shared callsite and the capture silently saw nothing (run 36424722859). Nothing
stops that pattern from returning. Second, the Lint job runs only on Linux,
where `cfg(windows)` code is not compiled. Clippy has never seen that code. A
cross-target run of `origin/main` finds 25 warnings that `-D warnings` would
reject:

- 17 in `kuru-memory`, `kuru-delivery` and `apps/kuru-tui`;
- 7 in `kuru-connectors`;
- 1 in `kuru-runtime`.

Several catalogued flakes lived in exactly that code.

This change is typed `fix`, not `ci`: the `ci` type forbids application source
edits, and the call-site expectations and Windows findings need them.

## What Changes

- A root `clippy.toml` bans thread-scoped and ad hoc global tracing subscriber
  installation through `disallowed-methods`. Every entry names the safe
  alternative in its reason. It is the first class check; later bans are
  separate changes.
- Each existing call site carries its own `#[expect(clippy::disallowed_methods,
  reason = "...")]` on the statement that makes the call. No crate- or
  module-level allowance is added.
- The `kuru-delivery` repository check rejects anything that would silently
  switch the ban off:
  - a `clippy.toml` or `.clippy.toml` below the root;
  - an allowance of the lint or of a group containing it wider than one
    statement: crate, module, `fn`, `impl` or `trait`;
  - an outer `allow` of them at any scope, since only `expect` fails when
    stale, and an `expect` without a reason;
  - a Cargo `[lints]` table that lowers it;
  - `CLIPPY_CONF_DIR` or a command-line `-A` in mise, workflow or any
    `.cargo/config{,.toml}` configuration, including `NAME=-A...` inside a
    command;
  - a root file entry without a reason.
- `kuru-memory`, `kuru-delivery` and `apps/kuru-tui` get package-owned
  `lint:windows` tasks. So do `kuru-connectors` and `kuru-runtime`:
  - Clippy's `-D warnings` also reaches the library code of the TUI's path
    dependencies, so five of connectors' findings had to be fixed anyway;
  - owning the task covers the remaining three test-target findings, so every
    package with `cfg(windows)` code is linted. They cross-lint for `x86_64-pc-windows-msvc` with
  `-D warnings`. On non-Windows hosts a delivery-owned stand-in C compiler lets
  the build scripts finish. It compiles no C, and every compile and probe
  succeeds; only Rust diagnostics are checked, and the native Windows jobs
  remain the build proof. A root
  `lint:windows` aggregate forwards to every package's task, including
  `kuru-platform`'s existing one.
- A CI job `Lint (x86_64-pc-windows-msvc)` runs on `ubuntu-latest`. It imports
  the run's verified bundle inputs offline, and `ci-gate` requires it.
- The 25 Windows findings on `main` are fixed or justified individually.
- AGENTS.md: the root workspace's role gains the shared lint configuration, and
  lint runs for the Windows target as well as the host.
  `docs/development.md` holds the operational detail.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- New: `clippy.toml`, `packages/kuru-delivery/support/windows-lint-cc`,
  `packages/kuru-delivery/src/repo/lints.rs`.
- Changed:
  - `packages/kuru-delivery/src/repo.rs` and
    `tests/repo_validation.rs`, `tests/release_workflow.rs`;
  - `.github/workflows/ci.yml`;
  - root `mise.toml` and five package `mise.toml` files;
  - the call sites in `apps/kuru-tui/src/diagnostics.rs`,
    `packages/kuru-connectors/src/auth.rs` and
    `packages/kuru-memory/src/provision/native_tests.rs`;
  - the files with Windows findings;
  - `AGENTS.md`, `docs/development.md`.
- No product behavior changes. `release.yml` does not run the new job. That is
  a release workflow change for the maintainer to decide.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
