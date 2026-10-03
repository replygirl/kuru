# Tasks

## 1. Regression test

- [ ] 1.1 Extract the existing inline `result?; cleanup?;` pattern into `finish` at all eight sites without changing behavior, add a `#[cfg(test)]` module covering all four arms, and verify the both-fail test fails against the old body (cleanup cause missing)

## 2. Fix

- [ ] 2.1 Combine both causes in `finish` as `engine.rs` shutdown does (primary first, then `<what> cleanup failed: …`, joined by `; `) and verify the unit tests pass
- [ ] 2.2 Keep stdout on primary success at the printing sites and the exact diagnostic-cleanup wording at the outer combine, and verify `mise run //apps/kuru-tui:test` passes (including `unix_shell_turn` diagnostic-cleanup assertions)

## 3. Checks and record

- [ ] 3.1 Run `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` and `mise run cospec -- validate --all --strict`, and record exit codes in verification.md
