## Why

Routine git/gh/mise/cargo actions in this repo currently prompt for approval
every time in both Claude Code and Codex, even for safe, everyday workflow
steps (status checks, commits, rebases, pushes with `--force-with-lease`,
opening PRs). Pre-approving the safe subset reduces prompt friction while
keeping irreversible or externally-visible actions (force push, push to
`main`, release/publish automation, repo settings changes) behind an explicit
one-time approval.

## What Changes

- Added project permission `allow`/`deny` rules to `.claude/settings.json`
  covering read-only git/gh/mise inspection; everyday git mutations (add,
  commit, switch, checkout -b, branch, worktree add/remove/prune, fetch,
  pull/merge --ff-only, rebase, cherry-pick, stash); `git push` including
  `--force-with-lease`; `gh pr`/`gh run` workflow commands; `mise run/
  install`; and `cargo build/test/check/clippy/fmt/llvm-cov/tree/metadata`.
  Added explicit deny rules for plain force push, push to `main`, remote
  branch deletion, `git reset --hard`, `gh pr merge --admin`,
  `gh release create/delete/edit`, `gh workflow run`, `gh repo`, and the
  kuru-delivery release/package/tool mise tasks (including the
  `release:tool`/`release:version`/`release:set-version` root aliases).
  `mise exec`/`mise x` was deliberately left off the allow list, even
  though it was in the original ask: it runs an arbitrary command inside
  mise's tool environment, so e.g. `mise exec -- git push --force origin
  main` would match only the `mise exec` prefix and bypass every git deny
  rule above — confirmed with `codex execpolicy check` and corrected
  before this change was committed.
- Added a new hand-authored Codex execpolicy rules file,
  `.codex/rules/agent-safe-actions.rules`, as a sibling of the
  cospec-managed `.codex/rules/cospec.rules`, mirroring the same scope with
  `prefix_rule(... decision="allow"|"forbidden")` entries. Because Codex's
  rule matcher compares whole argv tokens (not substrings), the mise section
  allowlists the specific routine task names AGENTS.md documents instead of
  a blanket `mise run` allow plus a deny list, which testing showed does not
  catch sibling task names such as `release:version`.
- Documented the distinction between these hand-authored permission files
  and cospec-managed files in `docs/development.md`.
- Did not add a project `.codex/config.toml`: it requires the user to mark
  the repository trusted in Codex's own security model, is unrelated to
  execpolicy `.rules` loading, and was not needed for this change.

## Impact

- `.claude/settings.json`, `.codex/rules/agent-safe-actions.rules`,
  `docs/development.md`. No application source, tests, or specs affected.
- Both files were validated command-by-command with
  `codex execpolicy check --rules ... -- <command>` before being committed.
