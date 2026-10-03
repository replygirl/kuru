# Verification

## 1. A failing command whose cleanup also fails reports both causes [critical]

- [ ] 1.1 @regression (agent) `cargo test -p kuru --lib cleanup_combination_tests` against the old inline body, then the fixed body -> red: the both-fail test fails because the joined message lacks the cleanup cause; green: both causes appear, primary before cleanup, joined by `; `
- [ ] 1.2 @unit (agent) the same module covers `(Ok, Ok)`, `(Ok, Err)`, `(Err, Ok)` -> success value preserved; a single failure is returned unchanged (same text, same chain)
- [ ] 1.3 @e2e (agent) a `kuru tool`/turn/dream run whose tool host or harness shutdown fails while the primary fails -> both causes on stderr

## 2. Existing CLI behavior is preserved

- [ ] 2.1 @integration (agent) `mise run //apps/kuru-tui:test` -> passes, including the `unix_shell_turn` assertion that a successful turn prints `diagnostic cleanup failed; diagnostics may be incomplete`

## 3. Static checks

- [ ] 3.1 @integration (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> each exits 0
