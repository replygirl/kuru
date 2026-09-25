## 1. Fix reference docs against code

- [x] 1.1 Add `grep`/`glob` to the native selector name list in
      `apps/kuru-docs/reference/configuration.md` and note `file_edit`'s
      shared `file_write` selector, verified against
      `packages/kuru-core/src/permissions.rs` (`NativeTool`) and
      `packages/kuru-connectors/src/tools.rs` (name → selector mapping)
- [x] 1.2 Add `offset`/`limit` to the `file_read` row and
      `include_hidden`/`include_ignored` to the `grep`/`glob` rows in
      `apps/kuru-docs/reference/tools.md`, and add prose on paging
      (`next_offset`, `omitted_lines`, limits) and on hidden/ignored search
      defaults and omission-count behavior, verified against
      `docs/protocols.md` and `packages/kuru-connectors/src/tools.rs`
- [x] 1.3 Add the missing `/compact [ID]` row to the slash-command table in
      `apps/kuru-docs/reference/commands.md`, verified against the
      `CommandSpec` for `CommandId::Compact` in `apps/kuru-tui/src/commands.rs`

## 2. Verify

- [x] 2.1 Run `mise run docs:build` and `mise run docs:check` and record
      pass/fail output
