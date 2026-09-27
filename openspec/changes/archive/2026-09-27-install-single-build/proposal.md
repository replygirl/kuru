# Proposal

## Why

The Windows native installation job compiles the release executable twice:
the source-install smoke (`mise run install`, 407 s) and then
`bundle:verify-native-build`, whose final "valid prepared archive" run rebuilds
kuru-memory, its dependents and the release link after two failed build-script
runs (449 s). That 20m25s job (main run 36308781079) is now the native run's
critical path. One offline release build per OS is enough if the install build
becomes the verifier's positive control, while both offline negatives keep
every assertion.

## What Changes

- `packages/kuru-memory/support/verify-bundle-build.ps1`: drop `--all-features`
  so the command equals the shipping `build:release` command (`--release
  --locked --target x86_64-pc-windows-msvc`, crt-static, plus `--offline`). Add a
  positive control before the negatives: the identical offline command against
  the caller's unchanged mirror setting must exit 0, print no `Compiling ` line,
  and leave `<target dir>/x86_64-pc-windows-msvc/release/kuru.exe` with the
  installed copy's SHA-256 (the file is only hashed). Keep the missing and
  same-size corrupt negatives and all their assertions. Remove the trailing
  valid rebuild, which the install build plus the control replace.
- `packages/kuru-memory/mise.toml`: update the task description; env unchanged.
- `.github/workflows/native-tests.yml`: comment only; no step, env, order or
  condition changes.
- `packages/kuru-delivery/tests/release_workflow.rs`: a static parity test
  between the verifier's Cargo command and the app-owned `build:release` task,
  plus the updated updater comment.
- `docs/development.md`: describe the control-then-negatives order and the
  build-script fingerprint left dirty in `target/`.
- No install path changes: `mise run install`, `install-source.ps1`,
  `scripts/install.*` and `kuru-delivery`'s `install-local` stay as they are.
  The smoke already copies Cargo's output through `archive::install_local`
  (`open_build_input`, `bounded`, `verify_build_snapshot`, `install_binary`).

## Impact

- Windows `Installation, offline runtime and update` job: one release compile
  instead of two; target about 13-14 min from 20m25s.
- Ubuntu and macOS install jobs already compile once (their smoke is the only
  release build; verify-native-build is Windows-only). Unchanged.
- No secrets, permissions, required-check names or job topology change. No
  product code changes. A local maintainer who runs the verifier leaves the
  kuru-memory build-script fingerprint dirty, so the next build reruns it.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
