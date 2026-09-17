## 1. Update published docs

- [x] 1.1 Rewrite `apps/kuru-docs/reference/configuration.md`'s "Tool permissions"
      section with the `[[permissions]]` array, selectors, precedence rule,
      once/session/always semantics, and an upgrade note for the
      `allow_write`/`allow_shell` behavior change; verify every sentence against
      `packages/kuru-core/src/permissions.rs` and
      `packages/kuru-connectors/src/permissions.rs`.
- [x] 1.2 Rewrite `apps/kuru-docs/reference/tools.md`'s built-in tools table and
      shell-authority paragraph to reflect the permission engine, the ask-not-block
      fallback, and headless structured permission-required denial; verify against
      `packages/kuru-connectors/src/tools.rs` and `permissions.rs`.
- [x] 1.3 Note the `[[permissions]]` MCP selector in `apps/kuru-docs/reference/mcp.md`.
- [x] 1.4 Grep `apps/kuru-docs` and `docs/` for other stale `allow_write`/
      `allow_shell`/`--allow-shell` statements; none found beyond the two files
      above (root `docs/configuration.md` and `docs/protocols.md` were already
      correct; other site mentions are accurate CLI-flag examples, not policy
      claims).

## 2. Verify

- [x] 2.1 Run `mise run //apps/kuru-docs:check` (format, lint, build, link/anchor
      content check) and `mise run format:check`; both pass.
