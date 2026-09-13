## Why

Built-in file reads and shell streams currently fail once their retained output
crosses 2 MiB, so a successful command or readable regular file can lose both
its useful beginning and its decisive ending. The runtime then applies a second,
head-only tool-receipt cut that can remove a producer tail before the next model
turn, despite retaining a valid call receipt.

## What Changes

- Replace the built-in file-read and independently captured shell stdout/stderr
  overflow failure with fixed-size, visible head-and-tail excerpts. The existing
  2 MiB shell budgets remain separate; execution deadlines and the checked
  regular-file boundary remain separate from retained-output handling.
- Stream recognized-secret projection before excerpt retention, keeping each
  visible redaction marker indivisible at a head/tail boundary. Reuse the
  connector scanner and its existing fixed limits; add no option, archive, or
  public result schema.
- Make existing runtime tool-receipt and optional tool-history truncation retain
  the excerpt tail while keeping JSON receipts valid and their call IDs intact.
- Keep MCP stdio, HTTP JSON, and SSE framing/parser admission limits unchanged.
  `file_list` retains its existing typed JSON result and 10,000-entry cap; this
  change does not widen any protocol record or add a listing schema.
- Update the bounded-output documentation and add focused connector, runtime,
  CLI, and required native Windows fixture evidence.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-tools`: Built-in tool output bounds must show a safe, bounded
  head-and-tail excerpt rather than failing solely because retained file or shell
  output crosses the visible budget.

## Impact

- `packages/kuru-connectors`: private scanner/retention helper, file and
  Unix/Windows shell capture, and connector fixtures.
- `packages/kuru-runtime`: existing tool-receipt and tool-history truncation
  call sites and their focused tests.
- `apps/kuru-tui`: direct tool-command evidence only; interactive activity stays
  metadata-only.
- `docs/protocols.md` and `apps/kuru-docs/reference/tools.md`: output-bound
  wording. MCP parser limits, dependencies, configuration, persistence, and
  public result schemas remain unchanged.

## Surfaces

- [x] interactive — direct CLI tool output shows the bounded excerpt.
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — model-facing tool receipts retain their call IDs and tails.
