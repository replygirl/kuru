## Why

Repository instructions should be shared across coding assistants, and Kuru's
cospec workflows should be available in Claude Code, Codex and OpenCode.

## What Changes

- Make `AGENTS.md` the canonical repository instruction source and import it
  from `CLAUDE.md` using Claude's `@AGENTS.md` syntax.
- Generate all three harness integrations with the pinned cospec CLI, keeping
  the existing mise and hk checks.
- Update the repository invariant and development documentation to describe
  the shared instruction source and generated workflow locations.

## Impact

Repository instructions, cospec-generated agent files, development documentation
and one tooling invariant change. Application behavior and dependencies stay
outside this chore.
