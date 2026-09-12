## 1. Authoritative completion [critical]

- [x] 1.1 @regression (agent) observe the pre-fix broadcast-response transcript failure, then assert post-fix authoritative completion facts -> expected: broadcast detail appended before; returned speaker, token counts and generic limited-result feedback after
- [x] 1.2 @integration (agent) dispatch a deterministic turn and deliver its result independently of dropped or misleading activity events -> expected: returned answer appears once with its own speaker and metadata
- [x] 1.3 @integration (agent) exercise stale completion generations, cancellation, slash commands and failures -> expected: no cancelled/stale answer and unchanged command/error feedback

## 2. Presentation and repository checks

- [x] 2.1 @e2e (agent) exercise real PTY completion at wide and narrow sizes and inspect rendered output -> expected: readable answer and completion facts without composer damage
- [x] 2.2 @integration (agent) run affected-package lint/typecheck and repository formatting -> observed `mise run format:fix`, `mise run format:check`, `mise run lint:rust`, and `mise run typecheck` exit 0 for the integrated workspace.
- [x] 2.3 @integration (agent) build and check public documentation -> observed `mise run docs:check` exits 0; VitePress builds and public links/content validation pass.
- [x] 2.4 @integration (agent) run the single workspace coverage suite -> observed `mise run coverage` exits 0 in 265.58 seconds and writes `target/coverage.lcov`; its enforced `--fail-under-lines 90` gate passes.

## Observed evidence

- 2026-09-11: `mise run //apps/kuru-tui:test -- --lib
  response_activity_never_becomes_transcript_content` failed on the pre-fix
  source at `apps/kuru-tui/src/ui.rs:1117`: the response broadcast detail had
  been appended to `View::transcript`. The same isolated real-memory fixture
  passed after the authoritative completion path replaced that behavior.
- 2026-09-11: focused real-memory regressions for returned relationship facts,
  late speaker/response activity, and wide/narrow metadata rendering passed.
  The renderer checks 120×35 and 60×20 cells and verifies returned text,
  input/output counts, and the generic limited-result indication.
- 2026-09-12: the isolated Unix PTY cancellation fixture passed at 35×120 and,
  after resize, 35×65. It preserved `Cancelled · turn interrupted`, excluded the
  late response, then rendered the accepted response once with aggregate returned
  usage (`32 input tokens · 20 output tokens`) and an intact composer. Focused
  View, rendering, slash-command/error, and eight-field `run --json` regressions
  also passed.
- 2026-09-12: integration formatting, Rust lint/typecheck, tooling, strict
  cospec, docs, and the single workspace coverage gate passed. Coverage includes
  the real terminal and visual fixtures and wrote `target/coverage.lcov`.
- Windows PTY verification is not run on this macOS host; it remains a native CI
  requirement.
