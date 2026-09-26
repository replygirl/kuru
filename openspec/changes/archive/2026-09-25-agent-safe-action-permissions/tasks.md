## 1. Claude Code permission rules

- [x] 1.1 Add allow/deny permission rules to `.claude/settings.json` for the
      agreed safe-action scope; verify with `jq . .claude/settings.json` and
      `python3 -c "import json; json.load(open('.claude/settings.json'))"`
      (both pass; 52 allow entries, 21 deny entries).

## 2. Codex execpolicy rules

- [x] 2.1 Determine which files under `.codex/` cospec manages, via
      `openspec/.cospec-manifest.json` and `mise run cospec -- update
      --check --json`; verified only `.codex/rules/cospec.rules` is listed,
      so a new sibling file is safe to hand-author.
- [x] 2.2 Add `.codex/rules/agent-safe-actions.rules` mirroring the Claude
      Code scope; verify every allow/forbidden branch with
      `codex execpolicy check --rules .codex/rules/agent-safe-actions.rules
      -- <command>` for representative commands (git push, git push
      --force, git push origin main, mise run build, mise run release,
      mise run release:version, gh pr merge --squash, gh pr merge --admin,
      gh release create). All matched the intended decision.
- [x] 2.3 Verify the new file and the cospec-managed `cospec.rules` load
      together without conflict via `codex execpolicy check --rules
      .codex/rules/cospec.rules --rules .codex/rules/agent-safe-actions.rules
      -- git status` (returns allow) and confirm `git diff --stat --
      .codex/rules/cospec.rules` is empty (untouched).

## 3. Documentation and validation

- [x] 3.1 Document the hand-authored-vs-cospec-managed distinction for
      these two files in `docs/development.md`.
- [x] 3.2 Run `mise run cospec:managed:check`; verify it passes (no drift
      reported for any cospec-managed file).
- [x] 3.3 Run `mise run lint:tooling`; verify it passes.
