## Why

Maintainers and agents build Kuru in many concurrent worktrees. A shared
`CARGO_TARGET_DIR` serializes them on one Cargo lock, while private targets
recompile the same dependencies repeatedly. mr-boxington (mbx) keeps each
checkout's own target directory and lock while sharing compiled outputs through
a content-addressed store.

## What Changes

- Pin `mr-boxington` exactly in root `mise.toml`, limited to the
  platforms with published mbx builds, and lock it in `mise.lock` for every
  locked platform.
- Enable mise's `mr_boxington` Cargo wrapper for the root Rust entry unless
  `KURU_MBX=0` is set or the host is Intel macOS, which has no mbx build.
- Raise the root mise `min_version` to 2026.9.4: the wrapper option needs
  2026.9.2, 2026.9.4 records packslip lock entries correctly for every platform,
  and it matches the mise version pinned in CI.
- Set `KURU_MBX=0` in CI workflows and in the source-installation and source
  update paths, so published and user-built binaries never use the maintainer
  cache.
- Keep each checkout's `target/` a real directory with `MBX_TARGET_VIEWS=0`,
  because prepared engine inputs and supervisor snapshots under it refuse
  symlinked paths; outputs are still shared through the mbx store.
- Check in a scheduler-only `.mbx.toml` that leaves CPU headroom for editors
  and real-memory fixtures across concurrent worktree builds.
- Accept the table form of the root Rust pin in the repository check while still
  requiring it to equal `rust-toolchain.toml`, and require exact root tool pins.
- Document the shared cache, its process-environment-only opt-out, the
  hard-link fallback on filesystems without cloning, and the removal of
  per-user `CARGO_TARGET_DIR` overrides.

## Impact

Touches root `mise.toml`, `mise.lock`, `.mbx.toml`, the four CI
workflows' top-level environment, `scripts/install.sh`, `scripts/install.ps1`,
the Windows source update command in `apps/kuru-tui/src/cli.rs` with its
Windows fixture assertions, `packages/kuru-delivery/src/repo.rs` with its
validation tests, and development/installation documentation. CI builds keep
their current toolchain, target directories and caching. Instrumented coverage
remains outside the mbx cache because cargo-llvm-cov supplies its own
`RUSTC_WRAPPER`, which mbx defers to.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
