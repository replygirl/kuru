## Why

Three generated-docs reference pages had drifted from the code: the permission
selector list omitted `grep`/`glob`, the tool argument table omitted paging and
hidden/ignored arguments the code and protocol docs already document, and the
slash-command table was missing `/compact`.

## What Changes

- `apps/kuru-docs/reference/configuration.md`: add `grep`/`glob` to the native
  selector name list and note that `file_edit` shares the `file_write`
  selector.
- `apps/kuru-docs/reference/tools.md`: add `offset`/`limit` to the `file_read`
  row, add `include_hidden`/`include_ignored` to the `grep`/`glob` rows, and
  add concise prose on paging (`next_offset`, `omitted_lines`, limits) and on
  hidden/ignored search defaults and omission-count behavior.
- `apps/kuru-docs/reference/commands.md`: add the missing `/compact [ID]`
  slash-command row.

## Impact

Readers of the CLI/TUI reference docs get an accurate native-tool permission
selector list, tool argument reference, and slash-command table. No code,
schema, or behavior changes.
