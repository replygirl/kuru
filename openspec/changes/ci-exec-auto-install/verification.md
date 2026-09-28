# Verification

## 1. CI downloads only each job's selected tools [critical]

- [x] 1.1 @integration (agent) isolated scratch mise project (local mise 2026.9.13, fresh data dir) configuring one uninstalled aqua tool: run `mise x -- true`, the command Windows `mise-shim.exe` issues, with and without `MISE_EXEC_AUTO_INSTALL=false` -> with the setting: exit 0, nothing installed, `mise ls` still reports `aqua:rhysd/actionlint 1.7.12 (missing)`; default: `installing 1 tool` and actionlint downloaded, exit 0. Pinned 2026.9.4 source agrees: `crates/mise-shim/src/main.rs` runs `mise x -- <tool>`, and `src/cli/exec.rs` skips installation when `exec_auto_install` is false
- [x] 1.2 @regression (agent) every workflow using `jdx/mise-action` sets `MISE_EXEC_AUTO_INSTALL: "false"` at workflow level, and `mise run lint:tooling` (actionlint, shellcheck, delivery repository checks) passes -> exactly one line in each of ci, quality, native-tests, bundle-build and release (pr-title uses no mise); lint:tooling exit 0, actionlint clean, "Repository metadata invariants passed."; `cargo test -p kuru-delivery --features tooling --test release_workflow`, which parses the workflows, passed 29/29 with the delivery package's cocogitto and communique pins supplied through `mise exec`
- [~] 1.3 @runtime (human) GitHub-hosted windows-latest native coverage partition under mise 2026.9.4: the `rustup component add` step prints no `installing N tools` line, never requests an mr-boxington asset, and the partition runs its tests -> defer: needs a CI run of the pushed branch, which this change's author was not authorized to push or trigger

## 2. Local development and source installation are unchanged

- [x] 2.1 @integration (agent) `mise ls --current` lists `mr-boxington 1.18.0` both with `KURU_MBX=0` and with it unset, and `mise.toml` and `mise.lock` have no diff -> both resolutions list mr-boxington 1.18.0 and the other seven root tools; `git diff --quiet -- mise.toml mise.lock` succeeds
- [x] 2.2 @manual (agent) source installers stay safe under `MISE_EXEC_AUTO_INSTALL=false`, which stops `exec` from installing anything, including a named tool -> `scripts/install.sh:16-17`, `apps/kuru-tui/support/install-source.ps1:20-22` and `kuru update --source` (`apps/kuru-tui/src/cli.rs`) each run an explicit `mise install rust` before `mise exec rust -- rustc`, so `rust` is already installed by the time `exec` runs; `exec` never needs to install it, and task auto-install is already off

## 3. Documentation

- [x] 3.1 @regression (agent) `mise run docs:check` and `mise run format:check` pass with the Shared build cache addition -> docs:check exit 0 ("Public docs artifacts, local links and anchors passed"); format:check exit 0
