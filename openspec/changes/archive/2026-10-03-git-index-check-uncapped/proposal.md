# Proposal

## Why

Before it accepts a discovered `.kuru/config.local.toml` as user authority, the
CLI asks Git whether the file is tracked (`git ls-files --error-unmatch`). That
query ran under a fixed 5 s `tokio::time::timeout`, so on a large index or a slow
disk every command in the repository aborted with "project-local Git index check
timed out". Unlike its sibling failures, that message offered no `--config`
remedy, and the 5 s figure had no derivation from anything the check observes.

## What Changes

- The index check awaits the Git child's exit without a ceiling. A silent
  `ls-files` (stdout and stderr are discarded) offers no progress signal other
  than its exit, so there is no evidence from which a ceiling could be derived;
  a kept number would only reintroduce the false abort.
- No ceiling is needed to stay bounded in practice: the known hang sources are
  already removed before the spawn (`--no-optional-locks`, `core.fsmonitor=false`,
  hooks pointed at a nonexistent path, every inherited `GIT_*` overlay removed),
  `kill_on_drop(true)` reaps the child when the enclosing future is dropped, and
  the user keeps their own cancel (interrupting the command).
- The "project-local Git index check timed out" message is removed. A spawn
  failure still reports "project-local Git index check is unavailable", and the
  exit-status mapping (0 tracked, 1 untracked, anything else ambiguous with the
  `--config` remedy) is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `apps/kuru-tui/src/cli.rs` (`discovered_local`): timeout removed around the
  Git index query.
- `apps/kuru-tui/tests/cli.rs`: a regression test drives the real `kuru config`
  against a `git` that answers untracked only after pacing past the former cap.
- `apps/kuru-tui/tests/fixtures/slow_git.rs` and `apps/kuru-tui/Cargo.toml`: a
  small cross-platform fixture binary (`kuru-slow-git-fixture`, gated by the
  `test-support` feature like the existing fixtures) copied into a temporary
  PATH directory as `git`/`git.exe`.
- No user documentation names the removed message or the cap; no spec states it.
- Out of scope: the Windows source-build cap elsewhere in `cli.rs`.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
