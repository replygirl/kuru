## 1. Observe the real function-call stream shape

- [x] 1.1 Extend the `responses-stream-reconcile` trace with a redaction-safe `stage="function-call"` record at every function-call stream event carrying event kind, item ID, name presence, call-ID presence and arguments length only, and verify the four new field names are admitted in `apps/kuru-tui/src/diagnostics.rs`'s `SafeFields`.
- [x] 1.2 Build the binary and run one live `codex`-route turn that forces a tool call with `--debug`, and verify the ring records which event carries the name and call ID for a `function_call` item.
- [x] 1.3 Record the observed event sequence, counts and field presence as the change's ground truth, and verify the recorded shape is consistent with the exact live error.
- [x] 1.4 Add a unit test in `apps/kuru-tui/src/diagnostics.rs`'s own `mod tests` pinning the new admitted fields directly, and verify an `arguments` field on the same event is dropped from the ring.

## 2. Resolve function-call identity by item ID

- [x] 2.1 Decode `response.output_item.added` for `function_call` items and retain the announced `name` and `call_id` keyed by item ID, and verify a second announcement may repeat those values but never change them.
- [x] 2.2 Treat `response.function_call_arguments.done` as carrying arguments only, with an optional `name`, and verify the tool-name check in `reconcile_fragments` still runs when that event did carry one.
- [x] 2.3 Complete every `function_call` in the authoritative output from its own announcement before it is returned, and verify both a backend-supplied terminal array and an output synthesized from streamed items are covered.
- [x] 2.4 Keep the hard error at dispatch time — no name or call ID ever announced, an announcement the terminal output contradicts, or arguments absent or not complete JSON — and verify each case still fails the turn.

## 3. Fixtures

- [x] 3.1 Add an SSE fixture mirroring the captured live event sequence, ids, field presence and empty terminal array, and verify it fails on the pre-fix decoder with `completed function arguments lack name` and passes after.
- [x] 3.2 Add the adjacent cases — arguments settled before the item's completed announcement, a name announced only on `added`, a name never announced, a contradicted name, and arguments that never became complete JSON — and verify with `cargo test -p kuru-connectors --lib providers::sse`.

## 4. Verify and document

- [x] 4.1 Run one live `codex`-route tool-calling turn with the fix on the branch build, and verify the tool actually ran and the turn produced an answer.
- [x] 4.2 Align `docs/protocols.md` with the announced-identity and dispatch rules, and verify with `mise run docs:check`.
- [x] 4.3 Run the package checks and verify with `mise run //packages/kuru-connectors:test`, `mise run //packages/kuru-runtime:test -- stream`, `mise run //apps/kuru-tui:test -- stream`, `mise run //apps/kuru-tui:test -- diagnostics`, `mise run format:check`, `mise run //packages/kuru-connectors:lint` and `mise run typecheck`.
