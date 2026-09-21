## 1. Observe the real stream shape

- [x] 1.1 Add a permanent redaction-safe `tracing::debug!` at the reconciliation point recording, for every streamed and terminal item, its ID, type, position and text length only, and verify the admitted field names are added to `SafeFields` in `apps/kuru-tui/src/diagnostics.rs` and that below-`INFO` records are gated on `--debug`.
- [x] 1.2 Build the binary and run one live `codex`-route turn with `--debug`, and verify the diagnostics ring records the streamed and terminal item shape for every peer.

## 2. Reconcile by item ID

- [x] 2.1 Match streamed items to a non-empty terminal `response.output` by item ID and let the terminal array own ordering and presence, and verify a reordered or dropped non-text item no longer fails the turn.
- [x] 2.2 Keep a hard error when a streamed item carrying user-visible text has no counterpart by ID in a non-empty terminal output, or its visible text disagrees, and verify both cases still error with their existing messages.
- [x] 2.3 Treat an empty terminal `output` array as absent when items were streamed, while keeping an empty array with nothing streamed an empty completion, and verify the existing empty-completion provider tests still pass.
- [x] 2.4 Locate every text, summary and tool fragment in `reconcile_fragments` by item ID instead of stream position, and verify text fragments still require a terminal item while summary and tool fragments skip an item the terminal output no longer carries.

## 3. Fixtures

- [x] 3.1 Add an SSE fixture mirroring the captured live event sequence, ids, indexes and empty terminal array, and verify it fails on the pre-fix decoder with `completed response omits streamed output` and passes after.
- [x] 3.2 Add the adjacent cases — reordered/dropped non-text items succeed, a missing or rewritten visible-text item errors, an empty terminal array with nothing streamed succeeds — and verify with `cargo test -p kuru-connectors --lib providers::sse`.

## 4. Verify and document

- [x] 4.1 Run one live `codex`-route turn with the fix, and verify stdout carries the answer and the committed turn records non-unknown input/output token usage.
- [x] 4.2 Align `docs/protocols.md`, `docs/usage.md` and `apps/kuru-docs/reference/commands.md` with the reconciliation rule and the new diagnostics detail, and verify with `mise run docs:check`.
- [x] 4.3 Run the package checks and verify with `mise run //packages/kuru-connectors:test`, `mise run //packages/kuru-runtime:test -- stream`, `mise run //apps/kuru-tui:test -- stream`, `mise run format:check`, `mise run //packages/kuru-connectors:lint` and `mise run typecheck`.
