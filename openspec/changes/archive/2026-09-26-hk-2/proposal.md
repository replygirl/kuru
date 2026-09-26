## Why

The repository's Git hooks run through hk 1.58.1, a major version behind the
current hk 2.2.0 release. Upgrading keeps the hook runner on a supported line
without changing which checks run before a commit or push.

## What Changes

- `mise.toml` / `mise.lock`: pin `aqua:jdx/hk` to `2.2.0` with refreshed
  per-platform lock entries, and remove the `HK_PKL_BACKEND = "pklr"`
  environment entry, which hk 2 documents as a compatibility no-op (pklr is
  always the evaluator). `HK_MISE = "1"` and the repository-scoped
  `hk install --mise` setup and postinstall hook stay unchanged; the machine-wide
  `hk install --global` launcher is deliberately not adopted.
- `hk.pkl`: amend the hk 2.2.0 `Config.pkl` package. Hook and step fields are
  unchanged; `pre-commit` stays check-only (`fix = false`), so hk 2's
  pre-commit auto-staging default does not apply, and `pre-push` keeps exactly
  the static steps from `pre-push-static`.
- `docs/dependencies.md`, `docs/release.md`: record the new pin and drop the
  stale version from the Intel macOS asset note (hk 2.2.0 still publishes no
  `x86_64-apple-darwin` archive).

## Impact

Local Git hooks and the hk tool pin only. Workflows install hk by name from the
lockfile (`install_args: rust aqua:jdx/hk`) and need no edit; jobs, secrets,
required checks and mise tasks are unchanged. No application source or durable
spec changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
