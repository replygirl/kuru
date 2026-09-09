## Why

Future sessions need accurate canonical instructions for the completed monorepo, cospec gates and release-only documentation flow. The current guide leaves task-scoped cospec invocation, artifact sizing and several repository-wide conventions implicit.

## What Changes

- Audit and update AGENTS.md against the actual package mise tasks, Cargo workspace, hooks, generated harnesses and release workflow.
- Preserve CLAUDE.md as an import of AGENTS.md and use documentation links for changing operational detail.

## Impact

Contributor and agent guidance only; no application or test changes. Verify all command examples and ownership claims against checked-in configuration, run managed-file drift and the repository gate, and archive this record before the final branch commit.
