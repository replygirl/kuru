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
      pass/fail output. **Correction:** the original pass recorded only the
      build/check exit status, not the rendered output; a passing build and
      `docs:check` is not evidence the page renders correctly (`docs:check`
      does not catch malformed tables). It missed two stray, unindented table
      rows left in `apps/kuru-docs/reference/tools.md` after the grep/glob
      paragraph, which rendered as raw `|`-delimited text inside a `<p>` in
      `.vitepress/dist/reference/tools.html`.
- [x] 2.2 Follow-up fix round: deleted the two stray rows; applied the same
      `grep`/`glob` selector fix to `docs/configuration.md` (the root
      contributor doc, distinct from `apps/kuru-docs/reference/configuration.md`,
      which already had it); added the missing `file_edit` row and reworded
      the "two file-mutation tools" intro in `tools.md`; reworded the
      `offset`/`limit` sentence to match `docs/protocols.md`'s "a limit
      outside 1-10,000 ... fails" phrasing; added grep/glob's numeric caps
      (10,000 candidate files, 2 MiB per file, 8 KiB per matched line, 2,000
      matches) and the per-candidate permission/no-whole-subtree-grant rule.
      Verified by rebuilding (`mise run docs:build` from the worktree, not
      the main checkout) and reading the built
      `.vitepress/dist/reference/{tools,configuration,commands}.html`
      directly: confirmed no `<p>` contains a bare `|`, the tools table has
      exactly 10 `<tr>` (1 header + 9 rows, `file_edit` included), and the
      `/compact [ID]` row is intact in `commands.html`.
